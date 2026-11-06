//! Roads on steep ground: the ground's heights levelled across the towns' roads, so a street on a
//! hillside is a flat shelf cut into it (with a bank beside it) instead of tilting with the slope.
//! terrain.rs `build_node` passes the heights it samples (vertices, the normal map, the parent's
//! heights for the geomorph) through [`NodeRoads::level`]; the ground's mesh, its shading, the scatter's
//! height pages (trees, buildings, cars and street furniture stand on them) and the camera's ground
//! all follow, as they all come from those heights. The roads are the ones planet.wesl draws
//! (city.wesl's layout, mirrored on the CPU in `se_core::procgen::city_roads`): a road levelled where
//! the shader draws none would be a terrace in a field, so keep the two in step.

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::FRAC_PI_2;
use std::sync::{Arc, Mutex, OnceLock};

use bevy::math::DVec3;
use se_core::planet_tiles::{CubeFace, TileKey, TileLayer};
use se_core::procgen::city_roads::{self, CityPos, NodeField};
use se_core::vfs::Vfs;

/// Nodes whose vertices lie closer than this (true m) level their roads; coarser ones keep their
/// heights (a 9-24 m road is a vertex or two there). The geomorph's parent heights follow the same
/// rule by the parent's spacing, so a node morphs exactly into the surface its parent drew.
pub const LEVEL_SPACING_M: f64 = 12.0;
/// Nodes this much coarser still level their normal maps (a texel is a quarter of a vertex), so the road's
/// shelf and banks come into the shading where they are a few pixels wide, not all at once at the split
/// into the first levelled nodes, where they are dozens.
pub const LEVEL_NORMALS_SPACING_M: f64 = 2.0 * LEVEL_SPACING_M;
/// The bank beside a road over which the levelling fades out (true m).
const BANK_M: f32 = 4.0;
/// Near a cube face's edge (true m) the levelling fades out: the layout is per face, so the two faces
/// would level the same edge by different roads, a step along the edge.
const FACE_EDGE_M: (f64, f64) = (4.0, 32.0);
/// The deepest level asked of a tile layer (its `source` stops at the tiles it has).
const PROBE_LEVEL: u8 = 24;
/// Decoded tiles kept for the levelling (a town's nodes want the same few).
const TILES_KEPT: usize = 48;

/// A tile's texels (bytes), decoded once for the levelling ([`LumCache`]).
pub struct LumTile {
    w: usize,
    h: usize,
    rgb: Vec<[u8; 3]>,
}

/// Decoded tiles by path (`None`: unreadable), shared by the node builds.
pub type LumCache = Arc<Mutex<HashMap<String, Option<Arc<LumTile>>>>>;

/// How a body's towns are found where it levels its roads.
#[derive(Clone)]
pub enum TownSource {
    /// A generated planet: the settlement field on the land that suits (planet.wesl's civ block).
    /// `lo`, `span` the ground's range, `sea` an ocean, `coast` the culture's liking for it, `test`
    /// 1 under `SE_CITY_TEST`, `airless` a colony whose towns stand under domes, `fert_min` the land's
    /// lowest fertility factor (`city_roads::generated_density`).
    Field { lo: f32, span: f32, sea: bool, coast: f32, test: f32, airless: bool, fert_min: f32 },
    /// A map body (Earth): the brightest channel of its lights map, off its water.
    Lights { lights: TileLayer, water: Option<TileLayer>, cache: LumCache },
}

/// What a node's build needs to level the roads of a body with a civilization of level 2 or more
/// (level 1's paths and tracks are left as they lie).
#[derive(Clone)]
pub struct RoadCtx {
    pub layout_seed: u32,
    pub field_seed: u32,
    pub level: u32,
    pub spread: f32,
    pub radius: f64,
    pub towns: TownSource,
}

/// city.wesl's metric at a node (true metres per face metre, the mean along u and v at its centre,
/// as terrain.rs writes `PlanetUniform::city_frame`).
pub fn node_metric(key: TileKey) -> f32 {
    let (u0, v0, s) = key.uv_rect();
    let (cu, cv) = (u0 + s * 0.5, v0 + s * 0.5);
    let e = s * 0.01;
    let du = (key.face.uv_to_dir(cu + e, cv) - key.face.uv_to_dir(cu - e, cv)).length() / (2.0 * e);
    let dv = (key.face.uv_to_dir(cu, cv + e) - key.face.uv_to_dir(cu, cv - e)).length() / (2.0 * e);
    (0.5 * (du / FRAC_PI_2 + dv / FRAC_PI_2)) as f32
}

fn srgb_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        std::array::from_fn(|i| {
            let c = i as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        })
    })
}

impl LumTile {
    fn decode(bytes: &[u8]) -> Option<LumTile> {
        let img = image::load_from_memory(bytes).ok()?.to_rgb8();
        let (w, h) = img.dimensions();
        Some(LumTile { w: w as usize, h: h as usize, rgb: img.pixels().map(|p| p.0).collect() })
    }

    /// Bilinear at `t` (0..1 over the tile), as planet.wesl samples a tile (`tile_uv`'s border,
    /// clamped at the edges): `value(texel)` for each of the four texels.
    fn sample(&self, t: (f64, f64), value: impl Fn(&[u8; 3]) -> f32) -> f32 {
        let x = (t.0 * (252.0 / 256.0) + 2.0 / 256.0) * self.w as f64 - 0.5;
        let y = (t.1 * (252.0 / 256.0) + 2.0 / 256.0) * self.h as f64 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let ix = |i: f64| (i.max(0.0) as usize).min(self.w - 1);
        let iy = |j: f64| (j.max(0.0) as usize).min(self.h - 1);
        let (xa, xb, ya, yb) = (ix(x0), ix(x0 + 1.0), iy(y0), iy(y0 + 1.0));
        let s = |i: usize, j: usize| value(&self.rgb[j * self.w + i]);
        let top = s(xa, ya) * (1.0 - fx) + s(xb, ya) * fx;
        let bot = s(xa, yb) * (1.0 - fx) + s(xb, yb) * fx;
        top * (1.0 - fy) + bot * fy
    }

    /// The brightest channel of the linear colour at `t`, the sRGB texels filtered in linear light as
    /// the GPU does: the three channels filtered apart, then the largest.
    fn lum(&self, t: (f64, f64)) -> f32 {
        let lut = srgb_lut();
        (0..3).map(|ch| self.sample(t, |c| lut[c[ch] as usize])).fold(0.0, f32::max)
    }

    /// The red channel at `t` as stored (a water mask: not sRGB).
    fn red(&self, t: (f64, f64)) -> f32 {
        self.sample(t, |c| c[0] as f32 / 255.0)
    }
}

/// The tile of `layer` holding face (`u`, `v`), remembered while a node's build stays on it.
struct TileLook<'a> {
    layer: &'a TileLayer,
    now: RefCell<Option<(TileKey, Option<Arc<LumTile>>)>>,
}

impl<'a> TileLook<'a> {
    fn new(layer: &'a TileLayer) -> Self {
        TileLook { layer, now: RefCell::new(None) }
    }

    /// The tile and the point's place in it.
    fn at(&self, face: CubeFace, u: f64, v: f64, cache: &LumCache, vfs: &Vfs) -> Option<(Arc<LumTile>, (f64, f64))> {
        let mut now = self.now.borrow_mut();
        let inside = |k: &TileKey| {
            let (au, av, asz) = k.uv_rect();
            u >= au && u < au + asz && v >= av && v < av + asz
        };
        if !now.as_ref().is_some_and(|(k, _)| inside(k)) {
            let n = 1u64 << PROBE_LEVEL;
            let probe = TileKey {
                face,
                level: PROBE_LEVEL,
                row: ((v.clamp(0.0, 1.0) * n as f64) as u64).min(n - 1) as u32,
                col: ((u.clamp(0.0, 1.0) * n as f64) as u64).min(n - 1) as u32,
            };
            let anc = self.layer.source(probe);
            let path = self.layer.tile_path(anc);
            let tile = {
                let mut c = cache.lock().unwrap();
                if let Some(t) = c.get(&path) {
                    t.clone()
                } else {
                    // (Decoded under the lock: the node builds of one town all want the same tile.)
                    let t = vfs.read(&path).ok().and_then(|b| LumTile::decode(&b)).map(Arc::new);
                    if c.len() >= TILES_KEPT {
                        c.clear();
                    }
                    c.insert(path, t.clone());
                    t
                }
            };
            *now = Some((anc, tile));
        }
        let (k, tile) = now.as_ref()?;
        let (au, av, asz) = k.uv_rect();
        Some((tile.clone()?, ((u - au) / asz, (v - av) / asz)))
    }
}

/// How a node finds its towns: the settlements that reach it, or its lights tiles.
enum NodeTowns<'a> {
    Field(NodeField),
    Lights { lights: TileLook<'a>, water: Option<TileLook<'a>> },
}

/// Road levelling for one node's build (one thread).
pub struct NodeRoads<'a> {
    ctx: &'a RoadCtx,
    vfs: &'a Vfs,
    face: CubeFace,
    /// The metric of the node the heights are for ([`node_metric`], as the shader's).
    k: f32,
    towns: NodeTowns<'a>,
}

impl RoadCtx {
    /// The ground's town density (0..1) at body-frame direction `dir` (unit), on ground `height` m above the
    /// radius: where it draws streets, so none over the sea or on land that doesn't suit. A map body's may
    /// read and decode a tile: not for the main thread.
    pub fn town_at(&self, dir: DVec3, height: f32, vfs: &Vfs) -> f32 {
        match &self.towns {
            TownSource::Field { lo, span, sea, coast, test, fert_min, .. } => {
                let d = dir.as_vec3().to_array();
                let st = NodeField::new(d, 0.05, self.radius as f32, self.level, self.field_seed).at(d);
                city_roads::generated_density(st, height, *lo, *span, *sea, *coast, *test, *fert_min).0
            }
            TownSource::Lights { lights, water, cache } => {
                let (face, u, v) = CubeFace::from_dir(dir);
                let Some((tile, t)) = TileLook::new(lights).at(face, u, v, cache, vfs) else { return 0.0 };
                let wet = water.as_ref().and_then(|w| TileLook::new(w).at(face, u, v, cache, vfs)).map_or(0.0, |(tile, t)| tile.red(t));
                city_roads::lights_density(tile.lum(t), wet).0
            }
        }
    }

    /// The levelling for node `key`, or `None` where no town can reach it (most of a planet: its
    /// heights stay as they are, at no cost).
    pub fn node<'a>(&'a self, key: TileKey, vfs: &'a Vfs) -> Option<NodeRoads<'a>> {
        let towns = match &self.towns {
            TownSource::Field { test, .. } => {
                let (_, _, s) = key.uv_rect();
                // A node's edge from its centre covers its corners and samples past them on any face.
                let node_km = (self.radius * FRAC_PI_2 * s / 1000.0) as f32 + 0.05;
                let field = NodeField::new(key.center_dir().as_vec3().to_array(), node_km, self.radius as f32, self.level, self.field_seed);
                if field.is_empty() && *test <= 0.0 {
                    return None;
                }
                NodeTowns::Field(field)
            }
            TownSource::Lights { lights, water, .. } => NodeTowns::Lights { lights: TileLook::new(lights), water: water.as_ref().map(TileLook::new) },
        };
        Some(NodeRoads { ctx: self, vfs, face: key.face, k: node_metric(key), towns })
    }
}

impl NodeRoads<'_> {
    /// Height `h` (m above the radius) at face (`u`, `v`), direction `dir`, levelled across the roads
    /// there: towards the height of the roads' middle lines across from the point (`h_at`, the ground's
    /// own heights), fully on a road and its sidewalks, fading out over the bank.
    pub fn level(&self, u: f64, v: f64, dir: DVec3, h: f64, h_at: &dyn Fn(f64, f64, DVec3) -> f64) -> f64 {
        let m = self.ctx.radius * FRAC_PI_2;
        // The face's edge: both faces keep the ground's own height there.
        let edge_m = u.min(1.0 - u).min(v).min(1.0 - v) * m * self.k as f64;
        if edge_m <= FACE_EDGE_M.0 {
            return h;
        }
        let cp = CityPos::from_face_m(u * m, v * m);
        let (seed, spread) = (self.ctx.layout_seed, self.ctx.spread);
        // The layout alone first (no density): most points lie by no road.
        let near = city_roads::roads_near(cp, self.k, 1.0, 1.0, seed, spread, BANK_M);
        if near.iter().all(Option::is_none) {
            return h;
        }
        // The densities at the point, and how to redo them at another height.
        let (town, rural, field) = match (&self.towns, &self.ctx.towns) {
            (NodeTowns::Field(f), TownSource::Field { lo, span, sea, coast, test, airless, fert_min }) => {
                let st = f.at(dir.as_vec3().to_array());
                let dome = if *airless { city_roads::dome_mask(cp, seed) } else { 1.0 };
                let at = move |height: f32| {
                    let (t, r) = city_roads::generated_density(st, height, *lo, *span, *sea, *coast, *test, *fert_min);
                    (t * dome, r)
                };
                let (t, r) = at(h as f32);
                (t, r, Some(at))
            }
            (NodeTowns::Lights { lights, water }, TownSource::Lights { cache, .. }) => {
                let Some((tile, t)) = lights.at(self.face, u, v, cache, self.vfs) else { return h };
                let wet = water.as_ref().and_then(|w| w.at(self.face, u, v, cache, self.vfs)).map_or(0.0, |(tile, t)| tile.red(t));
                let (town, rural) = city_roads::lights_density(tile.lum(t), wet);
                (town, rural, None)
            }
            _ => return h,
        };
        if town <= 0.0 && rural <= 0.0 {
            return h;
        }
        let mut spots = Vec::with_capacity(4);
        for mut s in city_roads::roads_near(cp, self.k, town, rural, seed, spread, BANK_M).into_iter().flatten() {
            let (x, y) = cp.face_m(s.centre);
            let (uc, vc) = (x / m, y / m);
            let hc = h_at(uc, vc, self.face.uv_to_dir(uc, vc));
            // A generated planet's land depends on the height, and the road's is its middle's: land that
            // suits at the point but not on the road gets no road from the shader.
            if let Some(at) = &field {
                let (t, r) = at(hc as f32);
                let keep = (s.kind.density(t, r) / s.density.max(1e-3)).min(1.0);
                s.w *= keep;
                s.share *= keep;
            }
            spots.push((s, hc));
        }
        let lev = city_roads::levelled(h, &spots);
        // (Smoothstep over the face's edge band.)
        let t = ((edge_m - FACE_EDGE_M.0) / (FACE_EDGE_M.1 - FACE_EDGE_M.0)).clamp(0.0, 1.0);
        h + (lev - h) * t * t * (3.0 - 2.0 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use se_core::procgen::civilization::nearest_settlement;

    /// The ground's own rule: inside a settlement's disc, a town stands on land that suits, not over its sea.
    #[test]
    fn a_town_stands_only_on_land_that_suits() {
        let (radius, seed) = (6.4e6, 1234);
        let field_seed = city_roads::field_seed(seed);
        let ctx = RoadCtx {
            layout_seed: city_roads::layout_seed(seed),
            field_seed,
            level: 2,
            spread: 0.0,
            radius,
            towns: TownSource::Field { lo: -5000.0, span: 10_000.0, sea: true, coast: 0.0, test: 0.0, airless: false, fert_min: 1.0 },
        };
        let vfs = Vfs::new();
        let (mid, _) = nearest_settlement(DVec3::X, radius, 2, field_seed, 2, 5000.0).expect("a settlement");
        assert!(ctx.town_at(mid, 200.0, &vfs) > 0.9, "{}", ctx.town_at(mid, 200.0, &vfs));
        assert_eq!(ctx.town_at(mid, 0.0, &vfs), 0.0, "over the sea");
        assert_eq!(ctx.town_at(mid, 4900.0, &vfs), 0.0, "high in the mountains");
    }
}
