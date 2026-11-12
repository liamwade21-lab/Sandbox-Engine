//! Solid buildings (spec F12.8, F6, F7, F2; step 9's second part). The probe (`scatter::probe`, step 9's first part) lists the
//! buildings the scatter has placed round him, in the body's frame, 3-5 frames late; boxes don't move, so he's solved against
//! the newest list every frame and walking feels no lag. Nothing is solid that isn't drawn: the list is what's drawn, and it
//! goes when the scatter stops drawing (`BuildingProbe`).
//!
//! - **Shapes** ([`shapes`]): each model (the box's `city.wesl` `M_*`) as up to `SLICES` stacked boxes, cut once from the very
//!   meshes the library draws (`library::building_mesh`): the heights split where they waste least room (a tower's terraces, a
//!   spire on its own), each slice's box round the mesh's points in it, turned where that fits much better (the twisted tower).
//!   The dome is a shell, an ellipsoid solid from both sides (its colony stands inside). Flying lanes are never solid: the probe
//!   never lists them.
//! - **Walls**: his sphere (`HIT_RADIUS_M` round his middle) against the slices that rise more than `ROOF_STEP_M` over his feet.
//!   A hit takes his velocity into the wall away and he slides on along it ([`Solids::slide`]); [`breaks`] is where step 10
//!   decides a hit breaks the building instead.
//! - **Roofs**: a slice whose top is no more than `ROOF_STEP_M` over his feet is ground ([`Solids::roof`], `ground::with_roof`):
//!   he lands on it, stands, walks and jumps there, and walks off its edge. Falling or flying, his middle can't go under a roof's
//!   top plus his stand height over it (`slide` with `floors`), whatever the ground's own trace found.
//! - **F2**: a building whose box holds him when it's first seen, or one he's found deep inside (a Go To or a camera's jump put
//!   him there), is let alone until he's out of it ([`Solids::refresh`]).

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy::math::{DVec2, DVec3, IVec2, Vec2, Vec3};
use bevy::prelude::*;
use se_core::catalog::ObjectRef;

use super::tune;
use crate::scatter::library::{BUILDING_MODELS, Mesh4, building_mesh};
use crate::scatter::probe::{MATCH_UNITS, ProbeBuilding};

/// `city.wesl` M_LANE and M_DOME.
pub const M_LANE: u8 = 10;
pub const M_DOME: u8 = 14;

/// One slice of a model in its unit box (x and z -0.5..0.5 scaled by the record's width and depth, y 0..1 by its height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    /// Its bottom and top (unit height).
    pub y: [f32; 2],
    /// Its turn about up, from the box's x towards its z (rad); in that turned frame its middle and half extents (unit).
    pub turn: f32,
    pub mid: Vec2,
    pub half: Vec2,
}

/// Every model's slices (the dome's and the lanes' are empty: a shell and nothing).
pub struct Shapes {
    pub models: Vec<Vec<Slice>>,
    /// The farthest a model's slices reach from its axis (unit footprint): for the quick tests.
    pub reach: Vec<f32>,
}

/// The shapes, cut the first time they're asked for (a few milliseconds; the hero models are read from their files).
pub fn shapes() -> &'static Shapes {
    static S: OnceLock<Shapes> = OnceLock::new();
    S.get_or_init(|| {
        let t = std::time::Instant::now();
        let mut models = Vec::with_capacity(BUILDING_MODELS);
        for mk in 0..BUILDING_MODELS {
            let slices = match mk as u8 {
                M_LANE | M_DOME => Vec::new(),
                // (A hero model whose file is missing is drawn as the box.)
                _ => building_mesh(mk).or_else(|| building_mesh(0)).map(|m| fit(&m, tune::SLICES)).unwrap_or_default(),
            };
            models.push(slices);
        }
        let reach = models
            .iter()
            .enumerate()
            .map(|(mk, s)| if mk as u8 == M_DOME { 0.5 } else { s.iter().map(slice_reach).fold(0.5f32, f32::max) })
            .collect();
        info!("viltrumite: buildings cut into slices in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        Shapes { models, reach }
    })
}

/// How far a slice's corners reach from the axis (unit footprint).
fn slice_reach(s: &Slice) -> f32 {
    let (c, si) = (s.turn.cos(), s.turn.sin());
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|&(a, b)| {
            let (u, v) = (s.mid.x + a * s.half.x, s.mid.y + b * s.half.y);
            Vec2::new(u * c - v * si, u * si + v * c).length()
        })
        .fold(0.0, f32::max)
}

/// Heights the cut may split at, at most (beyond the mesh's own when it has few).
const LEVELS: usize = 64;
/// Turns tried for a slice's box, over a quarter turn.
const TURNS: usize = 30;
/// A turned box is taken only if it's this much smaller than the plain one (or a round tower's would turn at random).
const TURN_GAIN: f32 = 0.9;

/// A mesh (unit box) cut into at most `max` slices: the heights chosen so the boxes waste the least room (a dynamic programme over
/// the mesh's own heights, with even steps between them when it has few), each box round the mesh's points in its band (the
/// triangles clipped to it), plain or turned.
pub fn fit(m: &Mesh4, max: usize) -> Vec<Slice> {
    let tris: Vec<[Vec3; 3]> = m.3.chunks_exact(3).map(|t| [0, 1, 2].map(|k| Vec3::from(m.0[t[k] as usize]))).collect();
    let (ylo, yhi) = m.0.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    let span = yhi - ylo;
    if tris.is_empty() || span <= 1e-6 || max == 0 {
        return Vec::new();
    }
    let mut own: Vec<f32> = m.0.iter().map(|p| p[1]).collect();
    own.sort_by(f32::total_cmp);
    own.dedup_by(|a, b| (*a - *b).abs() < span * 1e-4);
    let even = (0..=LEVELS).map(|k| ylo + span * k as f32 / LEVELS as f32);
    let mut levels: Vec<f32> = if own.len() <= 32 { own.into_iter().chain(even).collect() } else { even.collect() };
    levels.sort_by(f32::total_cmp);
    levels.dedup_by(|a, b| (*a - *b).abs() < span * 1e-3);
    let n = levels.len() - 1;
    let turns: Vec<(f32, f32)> = (0..TURNS).map(|k| (k as f32 / TURNS as f32 * std::f32::consts::FRAC_PI_2).sin_cos()).collect();
    // Per band and turn: the extents of its points (u min, u max, v min, v max), empty with u min > u max.
    const EMPTY: [f32; 4] = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
    let mut ext = vec![[EMPTY; TURNS]; n];
    for (i, e) in ext.iter_mut().enumerate() {
        let (y0, y1) = (levels[i], levels[i + 1]);
        let mut pts: Vec<Vec2> = Vec::new();
        for t in &tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if (y0..=y1).contains(&a.y) {
                    pts.push(a.xz());
                }
                for y in [y0, y1] {
                    if (a.y - y) * (b.y - y) < 0.0 {
                        pts.push(a.lerp(b, (y - a.y) / (b.y - a.y)).xz());
                    }
                }
            }
        }
        for (k, &(s, c)) in turns.iter().enumerate() {
            for p in &pts {
                let (u, v) = (p.x * c + p.y * s, -p.x * s + p.y * c);
                let x = &mut e[k];
                *x = [x[0].min(u), x[1].max(u), x[2].min(v), x[3].max(v)];
            }
        }
    }
    // Every run of bands as one box: its room (area x height), its turn and extents.
    let area = |x: &[f32; 4]| if x[0] > x[1] { 0.0 } else { (x[1] - x[0]) * (x[3] - x[2]) };
    let mut runs = vec![vec![(0.0f32, 0usize, EMPTY); n]; n];
    for i in 0..n {
        let mut acc = [EMPTY; TURNS];
        for j in i..n {
            for k in 0..TURNS {
                let (a, b) = (acc[k], ext[j][k]);
                acc[k] = [a[0].min(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].max(b[3])];
            }
            let best = (0..TURNS).min_by(|&a, &b| area(&acc[a]).total_cmp(&area(&acc[b]))).unwrap_or(0);
            let k = if area(&acc[best]) < area(&acc[0]) * TURN_GAIN { best } else { 0 };
            runs[i][j] = (area(&acc[k]) * (levels[j + 1] - levels[i]), k, acc[k]);
        }
    }
    // The fewest slices for the least room: each slice costs a little, so equal room takes fewer.
    let toll = span * 1e-4;
    let mut best = vec![vec![(f32::INFINITY, 0usize); n + 1]; max + 1];
    best[0][0] = (0.0, 0);
    for s in 1..=max {
        for j in 1..=n {
            for i in 0..j {
                let c = best[s - 1][i].0 + runs[i][j - 1].0 + toll;
                if c < best[s][j].0 {
                    best[s][j] = (c, i);
                }
            }
        }
    }
    let mut s = (1..=max).min_by(|&a, &b| best[a][n].0.total_cmp(&best[b][n].0)).unwrap_or(1);
    let mut j = n;
    let mut out = Vec::new();
    while s > 0 && j > 0 {
        let i = best[s][j].1;
        let (_, k, x) = runs[i][j - 1];
        if x[0] <= x[1] {
            let turn = k as f32 / TURNS as f32 * std::f32::consts::FRAC_PI_2;
            out.push(Slice {
                y: [levels[i], levels[j]],
                turn,
                mid: Vec2::new(0.5 * (x[0] + x[1]), 0.5 * (x[2] + x[3])),
                half: Vec2::new(0.5 * (x[1] - x[0]), 0.5 * (x[3] - x[2])),
            });
        }
        j = i;
        s -= 1;
    }
    out.reverse();
    out
}

/// A building, whichever list or record it came from: its face, model and id (spec F12.2, matched within `MATCH_UNITS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub face: u8,
    pub model: u8,
    pub id: IVec2,
}

impl Key {
    pub fn same(&self, o: &Key) -> bool {
        self.face == o.face && self.model == o.model && (self.id - o.id).abs().max_element() <= MATCH_UNITS
    }

    /// Its square of the matching grid (twice `MATCH_UNITS`, so a match lies in this or a neighbouring square).
    fn cell(&self) -> (u8, u8, i32, i32) {
        (self.face, self.model, self.id.x.div_euclid(2 * MATCH_UNITS), self.id.y.div_euclid(2 * MATCH_UNITS))
    }
}

/// A building of this frame, in the body's frame.
#[derive(Clone, Copy, Debug)]
pub struct Solid {
    pub key: Key,
    /// The probe's job kind (1 near, 2 far): step 10's restore and rubble read it.
    #[allow(dead_code)]
    pub kind: u8,
    /// Its foot (the plinth's bottom), axes (`za` = `xa` x `up`, as `vertex_building`'s) and size: width, depth, height (m).
    pub foot: DVec3,
    pub up: DVec3,
    pub xa: DVec3,
    pub za: DVec3,
    pub size: DVec3,
    /// Its middle, and how far its shape reaches from it (m): the quick tests.
    pub centre: DVec3,
    pub reach: f64,
    /// How far its slices reach from its axis (m).
    pub reach_xz: f64,
    /// F2: let alone until he's out of it.
    pub ignored: bool,
}

impl Solid {
    pub fn of(b: &ProbeBuilding) -> Option<Solid> {
        let size = b.size.as_dvec3();
        // (Kind 4 is a tree, listed for the sandbox's walker: `sandbox/solids.rs`.)
        if b.kind > 2 || b.model as usize >= BUILDING_MODELS || b.model == M_LANE || size.x < 0.1 || size.y < 0.1 || size.z < 0.1 || !b.foot.is_finite() {
            return None;
        }
        let reach_xz = shapes().reach[b.model as usize] as f64 * size.x.max(size.y);
        Some(Solid {
            key: Key { face: b.face, model: b.model, id: b.id },
            kind: b.kind,
            foot: b.foot,
            up: b.up,
            xa: b.xa,
            za: b.xa.cross(b.up),
            size,
            centre: b.foot + b.up * (0.5 * size.z),
            reach: (reach_xz * reach_xz + 0.25 * size.z * size.z).sqrt(),
            reach_xz,
            ignored: false,
        })
    }

    /// A point of the body's frame in its own (x along `xa`, y up from the foot, z along `za`), and a direction.
    fn local(&self, p: DVec3) -> DVec3 {
        let r = p - self.foot;
        DVec3::new(r.dot(self.xa), r.dot(self.up), r.dot(self.za))
    }

    fn local_dir(&self, d: DVec3) -> DVec3 {
        DVec3::new(d.dot(self.xa), d.dot(self.up), d.dot(self.za))
    }

    fn body_dir(&self, l: DVec3) -> DVec3 {
        self.xa * l.x + self.up * l.y + self.za * l.z
    }

    pub(crate) fn slices(&self) -> &'static [Slice] {
        &shapes().models[self.key.model as usize]
    }

    fn dome(&self) -> bool {
        self.key.model == M_DOME
    }

    /// The dome's semi-axes (m): across, up, along.
    fn semi(&self) -> DVec3 {
        DVec3::new(0.5 * self.size.x, self.size.z, 0.5 * self.size.y)
    }
}

/// A slice of a building in its frame (m): two upright slabs across the footprint (normals in x, z) and its height.
#[derive(Clone, Copy, Debug)]
struct Prism {
    slabs: [(DVec3, f64, f64); 3],
}

impl Prism {
    fn of(s: &Solid, sl: &Slice) -> Prism {
        let (w, d, h) = (s.size.x, s.size.y, s.size.z);
        let (si, c) = (sl.turn as f64).sin_cos();
        // u = (x / w) cos + (z / d) sin: a slab of the metres' frame whose normal is that gradient.
        let slab = |g: DVec2, mid: f32, half: f32| {
            let l = g.length().max(1e-12);
            (DVec3::new(g.x / l, 0.0, g.y / l), (mid - half) as f64 / l, (mid + half) as f64 / l)
        };
        Prism {
            slabs: [
                slab(DVec2::new(c / w, si / d), sl.mid.x, sl.half.x),
                slab(DVec2::new(-si / w, c / d), sl.mid.y, sl.half.y),
                (DVec3::Y, sl.y[0] as f64 * h, sl.y[1] as f64 * h),
            ],
        }
    }

    fn top(&self) -> f64 {
        self.slabs[2].2
    }

    /// Grown by `r` round (a box with square corners: a little more than the sphere's sweep), and its top raised by `lift`.
    fn grown(&self, r: f64, lift: f64) -> Prism {
        let mut p = *self;
        for (k, s) in p.slabs.iter_mut().enumerate() {
            s.1 -= r;
            s.2 += if k == 2 { r + lift } else { r };
        }
        p
    }

    /// Whether a point's footprint lies within it (m, its frame).
    fn over(&self, l: DVec3, margin: f64) -> bool {
        self.slabs[..2].iter().all(|(n, lo, hi)| {
            let s = n.dot(l);
            s >= lo - margin && s <= hi + margin
        })
    }
}

/// A segment against a convex shape.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cut {
    Miss,
    /// It starts inside: the shallowest way out (its frame) and how deep (m).
    Inside { n: DVec3, depth: f64 },
    /// It goes in at `t` (share of the segment), through the face whose outward normal is `n` (its frame).
    Hit { t: f64, n: DVec3 },
}

/// A segment from `a` along `d` (the whole of `d`) against slabs (Cyrus-Beck).
fn cut(p: &Prism, a: DVec3, d: DVec3) -> Cut {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    let mut n0 = DVec3::ZERO;
    for (ax, lo, hi) in &p.slabs {
        let (s0, ds) = (ax.dot(a), ax.dot(d));
        if ds.abs() < 1e-12 {
            if s0 < *lo || s0 > *hi {
                return Cut::Miss;
            }
            continue;
        }
        let (ta, tb) = ((lo - s0) / ds, (hi - s0) / ds);
        let (enter, exit, n) = if ta < tb { (ta, tb, -*ax) } else { (tb, ta, *ax) };
        if enter > t0 {
            t0 = enter;
            n0 = n;
        }
        t1 = t1.min(exit);
        if t0 > t1 {
            return Cut::Miss;
        }
    }
    if t1 < 0.0 || t0 > 1.0 {
        return Cut::Miss;
    }
    if t0 < 0.0 {
        let mut best = (f64::INFINITY, DVec3::Y);
        for (ax, lo, hi) in &p.slabs {
            let s0 = ax.dot(a);
            if s0 - lo < best.0 {
                best = (s0 - lo, -*ax);
            }
            if hi - s0 < best.0 {
                best = (hi - s0, *ax);
            }
        }
        return Cut::Inside { n: best.1, depth: best.0.max(0.0) };
    }
    Cut::Hit { t: t0, n: n0 }
}

/// A segment (its frame, m) against a dome's shell grown by `r` either side: from outside onto its outer face, from inside onto
/// its inner one (its colony stands inside). Only above its rim.
fn cut_dome(s: &Solid, a: DVec3, d: DVec3, r: f64) -> Cut {
    let semi = s.semi();
    let k = semi.min_element().max(1.0);
    let e = r / k;
    let (qa, dq) = (a / semi, d / semi);
    let fa = qa.length();
    let grad = |q: DVec3| (q / semi).normalize_or(DVec3::Y);
    if a.y < -r {
        return Cut::Miss;
    }
    if (fa - 1.0).abs() <= e {
        // In the shell's thickness: out the nearer side.
        let out = fa >= 1.0;
        let depth = if out { 1.0 + e - fa } else { fa - (1.0 - e) } * k;
        return Cut::Inside { n: if out { grad(qa) } else { -grad(qa) }, depth: depth.max(0.0) };
    }
    let outside = fa > 1.0;
    let radius = if outside { 1.0 + e } else { 1.0 - e };
    let (qa2, qd, dd) = (qa.length_squared() - radius * radius, qa.dot(dq), dq.length_squared());
    if dd < 1e-18 {
        return Cut::Miss;
    }
    let disc = qd * qd - dd * qa2;
    if disc < 0.0 {
        return Cut::Miss;
    }
    let root = disc.sqrt();
    // From outside the first root (going in); from inside the larger (going out).
    let t = if outside { (-qd - root) / dd } else { (-qd + root) / dd };
    if !(0.0..=1.0).contains(&t) || (outside && qd >= 0.0) {
        return Cut::Miss;
    }
    let q = qa + dq * t;
    if q.y < 0.0 {
        return Cut::Miss;
    }
    Cut::Hit { t, n: if outside { grad(q) } else { -grad(q) } }
}

/// A roof under a point: the radial distance along the point's direction to it, and its up (body frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roof {
    pub radial: f64,
    pub up: DVec3,
}

/// Something he met in a slide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// The wall's outward normal there, body frame.
    pub n: DVec3,
    /// His speed into it (m/s), before it was taken away.
    pub into: f64,
    /// Where his middle was when it touched (body frame).
    pub at: DVec3,
    /// Which building (this frame's list).
    pub solid: usize,
    /// A roof (or the dome's top) from above: ground.
    pub roof: bool,
}

/// What a slide came to.
#[derive(Clone, Copy, Debug, Default)]
pub struct Slid {
    pub end: DVec3,
    pub v: DVec3,
    /// The first roof he came down on, and the hardest wall.
    pub roof: Option<Contact>,
    pub wall: Option<Contact>,
    pub contacts: u32,
    /// Step 10: the buildings he broke this step (`breaks`): he went on through them, not slowed.
    pub broke: [Option<usize>; SLIDE_CONTACTS],
}

/// How a slide treats what it meets.
#[derive(Clone, Copy, Debug)]
pub struct SlideOpts {
    /// His sphere (m).
    pub r: f64,
    /// His middle over his feet (m).
    pub stand: f64,
    /// Falling or flying: roofs under his feet stop his middle at their top + `stand`. Walking they're his ground already.
    pub floors: bool,
    /// The most he's pushed out of a wall he's slightly inside, this step (m).
    pub push: f64,
}

/// Contacts a slide follows in one step, at most.
pub const SLIDE_CONTACTS: usize = 4;

/// Step 10 decides here whether a hit breaks the building (`into >= BREAK_SPEED_MS` along the wall's normal: it breaks and he isn't
/// slowed, T5). Until then nothing breaks: every hit stops him along the wall's normal and he slides.
pub fn breaks(c: &Contact) -> bool {
    c.into >= tune::BREAK_SPEED_MS
}

/// The buildings round him this frame (`Solids::refresh` from the probe's newest list), what F2 lets alone, and the measures.
#[derive(Resource, Default)]
pub struct Solids {
    /// The body they stand on: they're in its frame.
    pub obj: Option<ObjectRef>,
    pub list: Vec<Solid>,
    /// The probe's list they're from.
    pub seq: u32,
    /// F2: buildings that held him when first seen (or that he was found deep inside), until he's out of them.
    pub ignored: Vec<Key>,
}

/// What `hero_move` measures of the buildings (`hero solid stats`, `hero solid log`): frames with buildings round him, contacts,
/// roofs landed on, buildings let alone (F2), the hardest hit (m/s), and the deepest his middle ever was inside a box that wasn't
/// let alone (m: over 0 would be a building passed through).
#[derive(Default)]
pub struct SolidWatch {
    pub frames: u64,
    pub contacts: u64,
    pub roofs: u64,
    pub ignored: u64,
    pub hardest: f64,
    pub deepest: f64,
    pub fastest: f64,
    pub log: Option<std::io::BufWriter<std::fs::File>>,
}

impl Solids {
    /// Solid in this body's frame this frame.
    pub fn on(&self, obj: ObjectRef) -> bool {
        self.obj == Some(obj) && !self.list.is_empty()
    }

    /// Nothing solid.
    pub fn clear(&mut self) {
        self.obj = None;
        self.list.clear();
        self.ignored.clear();
        self.seq = 0;
    }

    /// From the probe's newest list (`None` without one, or one too old) and where his middle is (body and its frame). A list
    /// it hasn't seen is turned into this frame's buildings; one first seen holding him is let alone (F2), and so is one he's
    /// deep inside; those he's out of again count once more.
    /// Returns how many it let alone this time.
    pub fn refresh(&mut self, list: Option<(&[ProbeBuilding], u32, ObjectRef)>, him: Option<(ObjectRef, DVec3)>, broken: &[Key]) -> u32 {
        let mut newly = 0;
        let Some((recs, seq, obj)) = list else {
            self.clear();
            return 0;
        };
        let him = him.filter(|(o, _)| *o == obj).map(|(_, p)| p);
        if seq != self.seq || self.obj != Some(obj) {
            let mut seen: HashMap<(u8, u8, i32, i32), Vec<Key>> = HashMap::new();
            if self.obj == Some(obj) {
                for s in &self.list {
                    seen.entry(s.key.cell()).or_default().push(s.key);
                }
            } else {
                self.ignored.clear();
            }
            let was_seen = |k: &Key| {
                let (f, m, x, y) = k.cell();
                (-1..=1).any(|dx| (-1..=1).any(|dy| seen.get(&(f, m, x + dx, y + dy)).is_some_and(|v| v.iter().any(|o| o.same(k)))))
            };
            self.list = recs.iter().filter_map(Solid::of).collect();
            if let Some(p) = him {
                for s in &self.list {
                    if !was_seen(&s.key) && holds(s, p, tune::HIT_RADIUS_M) && !self.ignored.iter().any(|k| k.same(&s.key)) {
                        info!("viltrumite: a building (model {} id {} {}) held him when first seen: let alone until he's out of it (F2)", s.key.model, s.key.id.x, s.key.id.y);
                        self.ignored.push(s.key);
                        newly += 1;
                    }
                }
            }
            self.obj = Some(obj);
            self.seq = seq;
        }
        if let Some(p) = him {
            // Found deep inside one that held him in no first list: F2's case all the same.
            let deep: Vec<(Key, f64)> =
                self.list.iter().filter(|s| !self.ignored.iter().any(|k| k.same(&s.key))).map(|s| (s.key, depth_in(s, p))).filter(|(_, d)| *d > tune::DEEP_INSIDE_M).collect();
            for (k, d) in deep {
                info!("viltrumite: he's {d:.1} m inside a building (model {} id {} {}): let alone until he's out of it", k.model, k.id.x, k.id.y);
                self.ignored.push(k);
                newly += 1;
            }
            // Out of it: solid again.
            let list = &self.list;
            self.ignored.retain(|k| list.iter().any(|s| s.key.same(k) && holds(s, p, tune::HIT_RADIUS_M + 0.05)));
        } else {
            self.ignored.clear();
        }
        let ignored = &self.ignored;
        for s in self.list.iter_mut() {
            // (A broken building is nothing to him: it's falling or rubble, step 10.)
            s.ignored = ignored.iter().any(|k| k.same(&s.key)) || broken.iter().any(|k| k.same(&s.key));
        }
        newly
    }

    /// The highest roof under `p` (body frame) whose top is no more than `ROOF_STEP_M` over his feet, `feet_below` under `p`.
    pub fn roof(&self, p: DVec3, feet_below: f64) -> Option<Roof> {
        let dir = p.normalize_or(DVec3::Y);
        let mut best: Option<Roof> = None;
        for s in self.list.iter().filter(|s| !s.ignored) {
            let r = p - s.foot;
            if (r - s.up * r.dot(s.up)).length() > s.reach_xz + 0.01 {
                continue;
            }
            let l = s.local(p);
            let feet = l.y - feet_below + tune::ROOF_STEP_M;
            let mut top: Option<(f64, DVec3)> = None;
            if s.dome() {
                let semi = s.semi();
                let u = (l.x / semi.x).powi(2) + (l.z / semi.z).powi(2);
                if u < 1.0 {
                    let y = semi.y * (1.0 - u).sqrt();
                    if y <= feet {
                        top = Some((y, s.body_dir((DVec3::new(l.x, y, l.z) / (semi * semi)).normalize_or(DVec3::Y))));
                    }
                }
            } else {
                for sl in s.slices() {
                    let pr = Prism::of(s, sl);
                    let t = pr.top();
                    if t <= feet && top.is_none_or(|(y, _)| t > y) && pr.over(l, 0.0) {
                        top = Some((t, s.up));
                    }
                }
            }
            let Some((y, up)) = top else { continue };
            let along = dir.dot(s.up);
            if along < 0.5 {
                continue;
            }
            let radial = (y + s.foot.dot(s.up)) / along;
            if best.is_none_or(|b| radial > b.radial) {
                best = Some(Roof { radial, up });
            }
        }
        best
    }

    /// From `a` towards `b` (body frame, his middle), meeting what's in the way: each wall takes the motion and the velocity
    /// into it away and the rest goes on along it; with `floors` a roof under his feet stops his middle at its top + `stand`.
    /// Already a little inside a wall (a list's box moved onto him, he slid off a roof's edge), he's pushed out at most `push`
    /// and goes no further in; already under a roof's top + `stand`, he's lifted onto it (a step up).
    pub fn slide(&self, a: DVec3, b: DVec3, v: DVec3, o: SlideOpts) -> Slid {
        let mut out = Slid { end: a, v, ..default() };
        let (mut pos, mut target, mut v) = (a, b, v);
        let note = |out: &mut Slid, c: Contact| {
            out.contacts += 1;
            if c.roof {
                out.roof.get_or_insert(c);
            } else if out.wall.is_none_or(|w| c.into > w.into) {
                out.wall = Some(c);
            }
        };
        // What he's in already, each once.
        let mut inside: Vec<(DVec3, usize, Option<f64>, f64)> = Vec::new();
        self.visit(pos, DVec3::ZERO, o, |c, n, i, lift| {
            if let Cut::Inside { depth, .. } = c {
                inside.push((n, i, lift, depth));
            }
        });
        let mut pushed = 0.0;
        for (n, i, lift, depth) in inside {
            let into = -v.dot(n);
            let mv = target - pos;
            let shift = match lift {
                Some(l) => self.list[i].up * l,
                None => {
                    let push = (depth + 1e-4).min((o.push - pushed).max(0.0));
                    pushed += push;
                    n * push
                }
            };
            pos += shift;
            target = pos + mv - n * mv.dot(n).min(0.0);
            v -= n * v.dot(n).min(0.0);
            note(&mut out, Contact { n, into, at: pos, solid: i, roof: lift.is_some() });
        }
        let mut broke = 0;
        for _ in 0..SLIDE_CONTACTS {
            let d = target - pos;
            let mut first: Option<(f64, DVec3, usize, Option<f64>)> = None;
            let gone = out.broke;
            self.visit(pos, d, o, |c, n, i, lift| {
                if gone.contains(&Some(i)) {
                    return;
                }
                if let Cut::Hit { t, .. } = c {
                    if first.is_none_or(|(ft, ..)| t < ft) {
                        first = Some((t, n, i, lift));
                    }
                }
            });
            let Some((t, n, i, lift)) = first else {
                pos = target;
                break;
            };
            let into = -v.dot(n);
            let mut at = pos + d * t;
            at += match lift {
                // A roof: his middle at its top + `stand`, along the building's up.
                Some(l) => self.list[i].up * l,
                None => n * 1e-4,
            };
            let contact = Contact { n, into, at, solid: i, roof: lift.is_some() };
            note(&mut out, contact);
            if breaks(&contact) {
                // Step 10: the building breaks and he goes on through it, not slowed (T5).
                if broke < SLIDE_CONTACTS {
                    out.broke[broke] = Some(i);
                    broke += 1;
                }
                pos = at;
                continue;
            }
            let rest = target - at;
            target = at + rest - n * rest.dot(n).min(0.0);
            v -= n * v.dot(n).min(0.0);
            pos = at;
        }
        out.end = pos;
        out.v = v;
        out
    }

    /// Every slice (or dome) a move from `a` by `d` (body frame) cuts, with the normal in the body frame, the building, and for a roof
    /// how far up his middle goes there. Walls are grown by his sphere; roofs under his feet (with `floors`) are their footprint up
    /// to their top + `stand`; walking, those are his ground and left out.
    fn visit(&self, a: DVec3, d: DVec3, o: SlideOpts, mut f: impl FnMut(Cut, DVec3, usize, Option<f64>)) {
        for (i, s) in self.list.iter().enumerate() {
            if s.ignored || seg_dist(s.centre, a, d) > s.reach + o.r + o.stand {
                continue;
            }
            let (la, ld) = (s.local(a), s.local_dir(d));
            let feet = la.y - o.stand + tune::ROOF_STEP_M;
            let at_of = |c: &Cut| match c {
                Cut::Hit { t, .. } => la + ld * *t,
                _ => la,
            };
            if s.dome() {
                let semi = s.semi();
                let u = (la.x / semi.x).powi(2) + (la.z / semi.z).powi(2);
                let over = u < 1.0 && semi.y * (1.0 - u).max(0.0).sqrt() <= feet;
                if over && !o.floors {
                    // Walking on it: it's his ground.
                    continue;
                }
                let c = cut_dome(s, la, ld, o.r);
                let n = match c {
                    Cut::Hit { n, .. } | Cut::Inside { n, .. } => n,
                    Cut::Miss => continue,
                };
                // From above, on its top: a roof, his middle `stand` over the shell along the building's up.
                let lift = (over && n.y > 0.7).then(|| {
                    let at = at_of(&c);
                    let u = (at.x / semi.x).powi(2) + (at.z / semi.z).powi(2);
                    (semi.y * (1.0 - u).max(0.0).sqrt() + o.stand - at.y).max(0.0)
                });
                f(c, s.body_dir(if lift.is_some() { DVec3::Y } else { n }), i, lift);
                continue;
            }
            for sl in s.slices() {
                let pr = Prism::of(s, sl);
                if pr.top() <= feet {
                    // Under his feet: a roof (flying or falling), or his ground already (walking).
                    if !o.floors {
                        continue;
                    }
                    let c = cut(&pr.grown(0.0, o.stand), la, ld);
                    if c == Cut::Miss {
                        continue;
                    }
                    let lift = (pr.top() + o.stand - at_of(&c).y).max(0.0);
                    f(c, s.up, i, Some(lift));
                } else {
                    let c = cut(&pr.grown(o.r, 0.0), la, ld);
                    if let Cut::Hit { n, .. } | Cut::Inside { n, .. } = c {
                        f(c, s.body_dir(n), i, None);
                    }
                }
            }
        }
    }

    /// The first building along a ray (body frame) within `reach`: how far, and which. (A punch's.)
    pub fn ray(&self, a: DVec3, dir: DVec3, reach: f64) -> Option<(f64, usize)> {
        let d = dir * reach;
        let mut best: Option<(f64, usize)> = None;
        for (i, s) in self.list.iter().enumerate() {
            if s.ignored || seg_dist(s.centre, a, d) > s.reach {
                continue;
            }
            let (la, ld) = (s.local(a), s.local_dir(d));
            let cuts: Vec<Cut> = if s.dome() { vec![cut_dome(s, la, ld, 0.0)] } else { s.slices().iter().map(|sl| cut(&Prism::of(s, sl), la, ld)).collect() };
            for c in cuts {
                let t = match c {
                    Cut::Hit { t, .. } => t,
                    Cut::Inside { .. } => 0.0,
                    Cut::Miss => continue,
                };
                if best.is_none_or(|(bt, _)| t * reach < bt) {
                    best = Some((t * reach, i));
                }
            }
        }
        best
    }

    /// How far along a segment (body frame) a sphere of `r` gets before a wall (share of it, 1 if clear): the camera's line of sight.
    pub fn clear_share(&self, a: DVec3, b: DVec3, r: f64) -> f64 {
        let d = b - a;
        let mut share: f64 = 1.0;
        for s in self.list.iter().filter(|s| !s.ignored) {
            if seg_dist(s.centre, a, d) > s.reach + r {
                continue;
            }
            let (la, ld) = (s.local(a), s.local_dir(d));
            if s.dome() {
                if let Cut::Hit { t, .. } = cut_dome(s, la, ld, r) {
                    share = share.min(t);
                }
                continue;
            }
            for sl in s.slices() {
                match cut(&Prism::of(s, sl).grown(r, 0.0), la, ld) {
                    Cut::Hit { t, .. } => share = share.min(t),
                    Cut::Inside { .. } => share = 0.0,
                    Cut::Miss => {}
                }
            }
        }
        share
    }

    /// How deep his middle is inside any box that isn't let alone (m, 0 outside all): over 0 is a building passed through.
    pub fn deepest(&self, p: DVec3) -> f64 {
        self.list.iter().filter(|s| !s.ignored).map(|s| depth_in(s, p)).fold(0.0, f64::max)
    }

    /// His sphere's room to the nearest wall (m; under 0 it overlaps one), and that building. (The tests' measure.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clearance(&self, p: DVec3, r: f64) -> Option<(f64, usize)> {
        let mut best: Option<(f64, usize)> = None;
        for (i, s) in self.list.iter().enumerate().filter(|(_, s)| !s.ignored) {
            let l = s.local(p);
            let d = if s.dome() {
                ((l / s.semi()).length() - 1.0).abs() * s.semi().min_element() - r
            } else {
                s.slices().iter().map(|sl| box_gap(&Prism::of(s, sl), l)).fold(f64::INFINITY, f64::min) - r
            };
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, i));
            }
        }
        best
    }
}

/// `hero solid stats` (what the buildings did to him so far, and the buildings round him now) | `solid log FILE | off` (one line a
/// frame: contacts, the hardest wall, his depth inside any box) | `solid reset`.
pub fn dev_command(world: &mut World, arg: &str) {
    let (verb, rest) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
    match verb.to_ascii_lowercase().as_str() {
        "stats" | "" => {
            let n = world.resource::<Solids>().list.len();
            let ignored = world.resource::<Solids>().ignored.len();
            let w = &world.resource::<super::Hero>().solid;
            info!(
                "hero solid: {n} buildings round him now ({ignored} let alone); over {} frames with buildings: {} contacts, {} roofs landed on, {} let alone (F2), hardest wall {:.1} m/s, fastest near {:.0} m/s, deepest inside a box {:.3} m",
                w.frames, w.contacts, w.roofs, w.ignored, w.hardest, w.fastest, w.deepest
            );
        }
        "reset" => {
            let mut h = world.resource_mut::<super::Hero>();
            let log = h.solid.log.take();
            h.solid = SolidWatch { log, ..default() };
        }
        "log" => {
            let mut h = world.resource_mut::<super::Hero>();
            h.solid.log = match rest {
                "" | "off" => None,
                path => match std::fs::File::create(path) {
                    Ok(f) => Some(std::io::BufWriter::new(f)),
                    Err(e) => {
                        warn!("hero solid log: {path}: {e}");
                        None
                    }
                },
            };
        }
        other => warn!("hero solid: unknown '{other}' (stats | reset | log FILE|off)"),
    }
}

impl SolidWatch {
    /// One frame's measures: how many buildings were round him, a step's contacts and roof, the hardest wall (m/s into it), his
    /// speed and his depth inside any box that isn't let alone.
    pub fn note(&mut self, round: usize, contacts: u32, roof: bool, wall_into: f64, speed: f64, depth: f64) {
        use std::io::Write;
        if round == 0 {
            return;
        }
        self.frames += 1;
        self.contacts += contacts as u64;
        self.roofs += roof as u64;
        self.hardest = self.hardest.max(wall_into);
        self.fastest = self.fastest.max(speed);
        self.deepest = self.deepest.max(depth);
        if let Some(f) = self.log.as_mut() {
            let _ = writeln!(f, "{round} {contacts} {} {wall_into:.2} {speed:.1} {depth:.3}", roof as u8);
        }
    }
}

/// A point's distance outside a prism (m, negative inside: the depth).
fn box_gap(p: &Prism, l: DVec3) -> f64 {
    let mut out = DVec3::ZERO;
    let mut inside = f64::INFINITY;
    for (k, (n, lo, hi)) in p.slabs.iter().enumerate() {
        let s = n.dot(l);
        out[k] = (lo - s).max(s - hi).max(0.0);
        inside = inside.min((s - lo).min(hi - s));
    }
    if out == DVec3::ZERO { -inside.max(0.0) } else { out.length() }
}

/// How deep a point is inside a building's slices (m; 0 outside). A dome is a shell: inside its thickness only.
fn depth_in(s: &Solid, p: DVec3) -> f64 {
    let l = s.local(p);
    if s.dome() {
        return 0.0;
    }
    s.slices().iter().map(|sl| (-box_gap(&Prism::of(s, sl), l)).max(0.0)).fold(0.0, f64::max)
}

/// Whether his sphere at `p` (body frame) overlaps a building's walls (a dome's shell).
fn holds(s: &Solid, p: DVec3, r: f64) -> bool {
    let l = s.local(p);
    if s.dome() {
        return matches!(cut_dome(s, l, DVec3::ZERO, r), Cut::Inside { .. });
    }
    s.slices().iter().any(|sl| matches!(cut(&Prism::of(s, sl).grown(r, 0.0), l, DVec3::ZERO), Cut::Inside { .. }))
}

/// A point's distance from a segment (start `a`, `d` its whole length).
fn seg_dist(c: DVec3, a: DVec3, d: DVec3) -> f64 {
    let t = if d.length_squared() > 0.0 { ((c - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (c - (a + d * t)).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 6.0e6;

    /// A building of model `mk` standing on a body of radius `R` at its "north pole", width, depth, height.
    fn building(mk: u8, x: f64, z: f64, w: f32, d: f32, h: f32, id: i32) -> ProbeBuilding {
        ProbeBuilding {
            id: IVec2::new(id, 0),
            face: 0,
            model: mk,
            kind: 1,
            foot: DVec3::new(x, R, z),
            up: DVec3::Y,
            xa: DVec3::X,
            size: Vec3::new(w, d, h),
            k: 1.0,
        }
    }

    fn solids(recs: &[ProbeBuilding], him: Option<DVec3>) -> Solids {
        let mut s = Solids::default();
        s.refresh(Some((recs, 1, ObjectRef::Body(0))), him.map(|p| (ObjectRef::Body(0), p)), &[]);
        s
    }

    fn opts(floors: bool) -> SlideOpts {
        SlideOpts { r: tune::HIT_RADIUS_M, stand: tune::STAND_M, floors, push: 0.05 }
    }

    #[test]
    fn the_box_is_one_slice_and_the_stepped_tower_keeps_its_terraces() {
        let sh = shapes();
        let b = &sh.models[0];
        assert_eq!(b.len(), 1, "{b:?}");
        assert!((b[0].half - Vec2::splat(0.5)).abs().max_element() < 1e-4 && b[0].y == [0.0, 1.0], "{b:?}");
        let stepped = &sh.models[5];
        for top in [0.42f32, 0.7, 0.88] {
            // (Heights are cut on a 1/64 grid.)
            assert!(stepped.iter().any(|s| (s.y[1] - top).abs() <= 1.0 / 64.0), "no slice ends at {top}: {stepped:?}");
        }
        assert!(stepped.len() <= tune::SLICES);
        // The spire has its own thin slice over the last roof.
        let top = stepped.last().unwrap();
        assert!(top.half.max_element() < 0.1, "{top:?}");
        // The twisted tower's slices are turned with its twist.
        let twisted = &sh.models[6];
        assert!(twisted.iter().any(|s| s.turn > 0.2), "{twisted:?}");
        // The lanes are never solid, the dome is a shell.
        assert!(sh.models[M_LANE as usize].is_empty() && sh.models[M_DOME as usize].is_empty());
    }

    #[test]
    fn a_turned_slice_fits_the_twisted_tower_better() {
        let m = building_mesh(6).unwrap();
        let turned = fit(&m, 8);
        let area: f32 = turned.iter().map(|s| 4.0 * s.half.x * s.half.y * (s.y[1] - s.y[0])).sum();
        // The same heights with no turn.
        let plain: f32 = {
            let mut a = 0.0;
            for s in &turned {
                let (lo, hi) = m.0.iter().filter(|p| p[1] >= s.y[0] - 1e-4 && p[1] <= s.y[1] + 1e-4).fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| (lo.min(Vec2::new(p[0], p[2])), hi.max(Vec2::new(p[0], p[2]))));
                a += (hi - lo).x * (hi - lo).y * (s.y[1] - s.y[0]);
            }
            a
        };
        assert!(area < plain * 0.9, "turned {area} plain {plain}");
    }

    #[test]
    fn a_wall_stops_him_and_he_slides_along_it() {
        // A 20 m box at x 0..20 (its middle at 10), 60 m high.
        let s = solids(&[building(0, 10.0, 0.0, 20.0, 20.0, 60.0, 1)], None);
        // At 45 degrees into its -x wall (z -10..10), 10 m up, at 20 m/s for a second.
        let a = DVec3::new(-5.0, R + 10.0, -5.0);
        let v = DVec3::new(20.0, 0.0, 20.0) / 2f64.sqrt();
        let got = s.slide(a, a + v, v, opts(false));
        assert!(got.wall.is_some());
        assert!((got.end.x - (-tune::HIT_RADIUS_M)).abs() < 1e-3, "{:?}", got.end);
        assert!(got.end.z > a.z + 10.0, "slid on: {:?}", got.end);
        assert!(got.v.x.abs() < 1e-9 && (got.v.z - v.z).abs() < 1e-9, "{:?}", got.v);
    }

    #[test]
    fn at_thirty_km_s_he_never_passes_through() {
        let s = solids(&[building(0, 10.0, 0.0, 20.0, 20.0, 60.0, 1)], None);
        let a = DVec3::new(-400.0, R + 10.0, 3.0);
        let v = DVec3::new(30_000.0, 0.0, 0.0);
        let got = s.slide(a, a + v / 60.0, v, opts(true));
        // Step 10 (T5): over `BREAK_SPEED_MS` the building breaks and he goes on through it, not slowed.
        assert_eq!(got.broke[0], Some(0));
        assert!((got.wall.unwrap().into - 30_000.0).abs() < 1e-6);
        assert!((got.end - (a + v / 60.0)).length() < 1e-6, "{:?}", got.end);
        assert!((got.v - v).length() < 1e-9);
        // Under it he's stopped at the wall and nothing breaks.
        let slow = DVec3::new(30.0, 0.0, 0.0);
        let got = s.slide(DVec3::new(-1.0, R + 10.0, 3.0), DVec3::new(1.0, R + 10.0, 3.0), slow, opts(true));
        assert_eq!(got.broke, [None; SLIDE_CONTACTS]);
        assert!(got.end.x <= -tune::HIT_RADIUS_M + 1e-3, "{:?}", got.end);
        assert!(s.deepest(got.end) == 0.0);
    }

    #[test]
    fn a_roof_is_ground_within_its_step() {
        let s = solids(&[building(0, 0.0, 0.0, 20.0, 20.0, 60.0, 1)], None);
        let stand = tune::STAND_M;
        // Standing on it.
        let r = s.roof(DVec3::new(2.0, R + 60.0 + stand, 0.0), stand).expect("on it");
        assert!((r.radial - DVec3::new(2.0, R + 60.0, 0.0).length()).abs() < 1e-6 && r.up == DVec3::Y);
        // Feet 0.6 m under its top: still his ground (a step up).
        assert!(s.roof(DVec3::new(2.0, R + 59.4 + stand, 0.0), stand).is_some());
        // Feet 1 m under: a wall, not ground.
        assert!(s.roof(DVec3::new(2.0, R + 59.0 + stand, 0.0), stand).is_none());
        // Off its edge: nothing.
        assert!(s.roof(DVec3::new(10.5, R + 60.0 + stand, 0.0), stand).is_none());
    }

    #[test]
    fn falling_onto_a_roof_stops_his_middle_over_it() {
        let s = solids(&[building(0, 0.0, 0.0, 20.0, 20.0, 60.0, 1)], None);
        let a = DVec3::new(1.0, R + 70.0, 1.0);
        // (Under `BREAK_SPEED_MS`: faster, the building breaks, step 10.)
        let v = DVec3::new(0.0, -30.0, 0.0);
        let got = s.slide(a, a + DVec3::new(0.0, -15.0, 0.0), v, opts(true));
        let roof = got.roof.expect("a roof");
        assert!((got.end.y - (R + 60.0 + tune::STAND_M)).abs() < 1e-6, "{:?}", got.end);
        assert!((roof.into - 30.0).abs() < 1e-6 && got.v.y.abs() < 1e-9);
        // At 900 m/s it breaks and he falls on through (T5: a fall on a roof that makes a crater breaks the building).
        let fast = s.slide(a, a + DVec3::new(0.0, -15.0, 0.0), DVec3::new(0.0, -900.0, 0.0), opts(true));
        assert_eq!(fast.broke[0], Some(0));
    }

    #[test]
    fn a_building_holding_him_when_first_seen_is_let_alone_until_he_leaves() {
        let b = building(0, 0.0, 0.0, 20.0, 20.0, 60.0, 1);
        let inside = DVec3::new(0.0, R + 5.0, 0.0);
        let mut s = solids(&[b], Some(inside));
        assert!(s.list[0].ignored && s.ignored.len() == 1);
        // Inside it he moves freely.
        let got = s.slide(inside, inside + DVec3::X * 5.0, DVec3::X, opts(false));
        assert!(got.contacts == 0);
        // Out of it, it's solid again (the next list holds the same building, its id a unit off).
        let out = DVec3::new(-15.0, R + 5.0, 0.0);
        let mut b2 = b;
        b2.id.x += 1;
        s.refresh(Some((&[b2], 2, ObjectRef::Body(0))), Some((ObjectRef::Body(0), out)), &[]);
        assert!(!s.list[0].ignored && s.ignored.is_empty());
        let got = s.slide(out, out + DVec3::X * 10.0, DVec3::X, opts(false));
        assert!(got.wall.is_some() && got.end.x < -10.0 + 1e-3 - tune::HIT_RADIUS_M + 0.01);
        // Seen before, it holds him only by his going in: never let alone for that.
        let mut c = solids(&[b], Some(out));
        c.refresh(Some((&[b2], 2, ObjectRef::Body(0))), Some((ObjectRef::Body(0), DVec3::new(-10.2, R + 5.0, 0.0))), &[]);
        assert!(!c.list[0].ignored);
    }

    #[test]
    fn a_dome_is_solid_from_both_sides_and_its_top_is_ground() {
        let d = building(M_DOME, 0.0, 0.0, 300.0, 300.0, 120.0, 1);
        let s = solids(&[d], None);
        // From inside, flying up: stopped under the shell.
        let a = DVec3::new(0.0, R + 50.0, 0.0);
        let got = s.slide(a, a + DVec3::Y * 200.0, DVec3::Y * 30.0, opts(true));
        assert!(got.wall.is_some() && got.end.y < R + 120.0 - tune::HIT_RADIUS_M + 0.01, "{:?}", got.end);
        // From outside, falling on its top: a roof.
        let a = DVec3::new(0.0, R + 140.0, 0.0);
        let got = s.slide(a, a - DVec3::Y * 30.0, -DVec3::Y * 30.0, opts(true));
        assert!(got.roof.is_some(), "{got:?}");
        assert!((got.end.y - (R + 120.0 + tune::STAND_M)).abs() < 0.05, "{:?}", got.end);
        assert!(s.roof(DVec3::new(0.0, R + 120.0 + tune::STAND_M, 0.0), tune::STAND_M).is_some());
    }

    #[test]
    fn the_line_of_sight_stops_at_a_wall() {
        let s = solids(&[building(0, 10.0, 0.0, 20.0, 20.0, 60.0, 1)], None);
        let a = DVec3::new(-10.0, R + 2.0, 0.0);
        let share = s.clear_share(a, a + DVec3::X * 40.0, 0.3);
        assert!((share * 40.0 - 9.7).abs() < 1e-6, "{share}");
        assert!(s.ray(a, DVec3::X, 5.0).is_none() && s.ray(a, DVec3::X, 15.0).is_some_and(|(d, _)| (d - 10.0).abs() < 1e-6));
    }
}
