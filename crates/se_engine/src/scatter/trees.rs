//! The tree generator (the vegetation plan's step 4). A species is a recipe (trunk, branches, crown
//! shape, leaf cards) grown into a mesh: tapered tubes for the trunk and branches dressed in Megaton
//! Rainfall's bark, leaf cards at the branches (MR's broadleaf branch cards, the port's own conifer
//! and palm cards). Earth's conifers, palms, acacia and shrubs are fixed recipes beside MR's 16
//! broadleaf trees; every generated planet with land plants gets a flora of its own (8 species from
//! its seed, its gravity and its air). The trees' colour comes from the ground where they stand
//! (scatter_draw.wesl `inst_tint`), so a planet's palette dresses its flora.

use bevy::math::Vec3;

/// The crown's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crown {
    /// A rounded broadleaf crown.
    Broad,
    /// A conifer's cone of drooping tiers.
    Cone,
    /// A narrow column.
    Column,
    /// A flat crown on a bare trunk (acacia).
    Umbrella,
    /// A fan of fronds on a bare trunk.
    Palm,
    /// A low clump without a trunk.
    Shrub,
}

/// The leaf card a species wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leaf {
    /// One of MR's 8 broadleaf branch cards (`BRANCHES`).
    Branch(u8),
    /// The port's conifer branch card.
    Conifer,
    /// The port's palm frond card.
    Palm,
}

/// MR's bark sets and broadleaf branch cards (its `Textures/Trees`).
pub const BARKS: [&str; 9] = ["Trees/Bark1A", "Trees/Bark2A", "Trees/Bark3A", "Trees/Bark4A", "Trees/Bark5A", "Trees/Bark6A", "Trees/Bark7A", "Trees/Bark9A", "Trees/Bark10A"];
pub const BRANCHES: [&str; 8] = ["Trees/Branch1A", "Trees/Branch2A", "Trees/Branch3A", "Trees/Branch4A", "Trees/Branch5A", "Trees/Branch6A", "Trees/Branch7A", "Trees/Branch10"];

/// A species.
#[derive(Clone, Debug)]
pub struct Recipe {
    pub crown: Crown,
    /// Height (m) and the trunk's radius at its foot (m).
    pub height: f32,
    pub trunk_r: f32,
    /// Where the branches start (share of the height).
    pub crown_base: f32,
    /// The crown's radius over the height.
    pub spread: f32,
    /// Branches (fronds for a palm, cards for a shrub).
    pub branches: u32,
    /// The branches' angle up from level (rad) and how far they bend down towards their tips.
    pub tilt: f32,
    pub droop: f32,
    /// The leaf cards' size (m) and how many on a branch.
    pub card: f32,
    pub cards: u32,
    /// The trunk's lean at the top (m).
    pub bend: f32,
    /// Bark set (`BARKS`) and leaf card.
    pub bark: u8,
    pub leaf: Leaf,
    pub seed: u32,
}

/// A grown mesh: bark and leaves as two index ranges over one vertex list.
#[derive(Default, Clone)]
pub struct Grown {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub bark: Vec<u32>,
    pub leaves: Vec<u32>,
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 16_777_216.0
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

fn seeded(seed: u32) -> Rng {
    let mut r = Rng(seed.wrapping_mul(0x9e37_79b9) | 1);
    for _ in 0..4 {
        r.next();
    }
    r
}

impl Grown {
    fn vertex(&mut self, p: Vec3, n: Vec3, uv: [f32; 2]) -> u32 {
        self.positions.push(p.to_array());
        self.normals.push(n.normalize_or(Vec3::Y).to_array());
        self.uvs.push(uv);
        self.positions.len() as u32 - 1
    }

    /// A tube along `path` with `radii`, `sides` round (the bark repeats twice round it and keeps its
    /// aspect along it).
    fn tube(&mut self, path: &[Vec3], radii: &[f32], sides: u32) {
        let base = self.positions.len() as u32;
        let circ = std::f32::consts::TAU * radii[0].max(0.01);
        let mut v = 0.0;
        for (i, &p) in path.iter().enumerate() {
            let t = if i + 1 < path.len() { path[i + 1] - p } else { p - path[i - 1] }.normalize_or(Vec3::Y);
            let a = if t.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
            let e1 = t.cross(a).normalize();
            let e2 = t.cross(e1);
            if i > 0 {
                v += (p - path[i - 1]).length() / (circ * 1.0);
            }
            for s in 0..=sides {
                let ang = s as f32 / sides as f32 * std::f32::consts::TAU;
                let n = e1 * ang.cos() + e2 * ang.sin();
                self.vertex(p + n * radii[i], n, [s as f32 / sides as f32 * 2.0, v]);
            }
        }
        let ring = sides + 1;
        for i in 0..path.len() as u32 - 1 {
            for s in 0..sides {
                let a = base + i * ring + s;
                let b = a + ring;
                // Wound outwards: the bark is drawn with back faces culled.
                self.bark.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
    }

    /// A leaf card centred on `c`: its stem end at `c - up * size / 2`, its tip towards `up`, spanning
    /// `side`; the card texture's stem is at its bottom (v 1).
    fn card(&mut self, c: Vec3, up: Vec3, side: Vec3, size: f32, width: f32) {
        let (u, s) = (up.normalize_or(Vec3::Y) * size * 0.5, side.normalize_or(Vec3::X) * width * 0.5);
        let n = side.cross(up).normalize_or(Vec3::Y);
        let b = self.vertex(c - u - s, n, [0.0, 1.0]);
        self.vertex(c - u + s, n, [1.0, 1.0]);
        self.vertex(c + u - s, n, [0.0, 0.0]);
        self.vertex(c + u + s, n, [1.0, 0.0]);
        self.leaves.extend([b, b + 1, b + 2, b + 2, b + 1, b + 3]);
    }

    /// A palm frond from `root` along `dir`: a strip of `segs` pieces bent down by `droop`, folded
    /// into a shallow V along its stalk.
    fn frond(&mut self, root: Vec3, dir: Vec3, len: f32, width: f32, droop: f32) {
        let segs = 4;
        let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
        let base = self.positions.len() as u32;
        let mut p = root;
        let mut d = dir.normalize();
        for i in 0..=segs {
            let t = i as f32 / segs as f32;
            // Widest a third of the way out, narrow at the tip (the card's own shape does the rest).
            let w = width * 0.5 * (1.0 - 0.6 * t);
            let up = side.cross(d).normalize_or(Vec3::Y);
            let lift = up * w * 0.35;
            let n = up;
            self.vertex(p - side * w + lift, n, [0.0, 1.0 - t]);
            self.vertex(p, n, [0.5, 1.0 - t]);
            self.vertex(p + side * w + lift, n, [1.0, 1.0 - t]);
            if i < segs {
                p += d * (len / segs as f32);
                d = (d - Vec3::Y * droop * 0.6).normalize();
            }
        }
        for i in 0..segs as u32 {
            let a = base + i * 3;
            let b = a + 3;
            self.leaves.extend([a, a + 1, b, a + 1, b + 1, b, a + 1, a + 2, b + 1, a + 2, b + 2, b + 1]);
        }
    }
}

/// Grows `r` into a mesh (y up, its foot at the origin).
pub fn grow(r: &Recipe) -> Grown {
    let mut g = Grown::default();
    let mut rng = seeded(r.seed);
    let h = r.height;
    let lean_dir = {
        let a = rng.range(0.0, std::f32::consts::TAU);
        Vec3::new(a.cos(), 0.0, a.sin())
    };
    // The trunk's centre line at height share t.
    let trunk_top = match r.crown {
        Crown::Palm => 1.0,
        Crown::Shrub => 0.35,
        Crown::Umbrella => 0.8,
        _ => 0.92,
    };
    let centre = |t: f32| lean_dir * (r.bend * t * t) + Vec3::Y * (t * h);
    let taper = |t: f32| if r.crown == Crown::Palm { 1.0 - 0.35 * t } else { 1.0 - 0.8 * t };
    if r.crown == Crown::Shrub {
        // A few short stems.
        for k in 0..3 {
            let a = k as f32 * 2.1 + rng.range(0.0, 0.6);
            let d = Vec3::new(a.cos() * 0.35, 1.0, a.sin() * 0.35).normalize();
            let path: Vec<Vec3> = (0..3).map(|i| d * (i as f32 * h * 0.18)).collect();
            g.tube(&path, &[r.trunk_r, r.trunk_r * 0.6, r.trunk_r * 0.25], 4);
        }
    } else {
        let segs = 6;
        let path: Vec<Vec3> = (0..=segs).map(|i| centre(i as f32 / segs as f32 * trunk_top)).collect();
        let radii: Vec<f32> = (0..=segs).map(|i| r.trunk_r * taper(i as f32 / segs as f32 * trunk_top)).collect();
        g.tube(&path, &radii, 7);
    }
    let golden = 2.399_963_f32;
    match r.crown {
        Crown::Palm => {
            let top = centre(1.0);
            for i in 0..r.branches {
                let a = i as f32 * golden + rng.range(-0.2, 0.2);
                let up = rng.range(r.tilt * 0.6, r.tilt * 1.4);
                let d = Vec3::new(a.cos() * up.cos(), up.sin(), a.sin() * up.cos());
                let len = r.spread * h * rng.range(0.8, 1.15);
                g.frond(top, d, len, r.card, r.droop * rng.range(0.7, 1.3));
            }
        }
        Crown::Shrub => {
            let c = Vec3::Y * (h * 0.5);
            let rad = r.spread * h;
            for _ in 0..r.branches {
                let a = rng.range(0.0, std::f32::consts::TAU);
                let y = rng.range(-0.6, 1.0);
                let out = Vec3::new(a.cos() * (1.0 - y * y).sqrt(), y, a.sin() * (1.0 - y * y).sqrt());
                let p = c + Vec3::new(out.x * rad, out.y * h * 0.45, out.z * rad) * rng.range(0.55, 1.0);
                let up = (out + Vec3::Y * 0.8).normalize();
                let side = up.cross(out).normalize_or(Vec3::X);
                g.card(p, up, side, r.card, r.card);
                g.card(p, up, out.cross(Vec3::Y).normalize_or(Vec3::Z) * 0.7 + side * 0.3, r.card, r.card);
            }
        }
        _ => {
            let n = r.branches.max(1);
            for i in 0..n {
                let u = (i as f32 + rng.range(0.2, 0.8)) / n as f32;
                let t = r.crown_base + (trunk_top - r.crown_base) * u;
                // How long the branches are at that height: the crown's outline.
                let env = match r.crown {
                    Crown::Cone => (1.0 - u).powf(0.85) * 0.95 + 0.05,
                    Crown::Column => 0.55 * (1.0 - 0.6 * u),
                    Crown::Umbrella => 0.55 + 0.45 * u,
                    _ => (std::f32::consts::PI * (0.15 + 0.85 * u)).sin().powf(0.6),
                };
                let len = (r.spread * h * env * rng.range(0.8, 1.15)).max(0.3);
                let a = i as f32 * golden + rng.range(-0.3, 0.3);
                let tilt = r.tilt + rng.range(-0.15, 0.15);
                let d = Vec3::new(a.cos() * tilt.cos(), tilt.sin(), a.sin() * tilt.cos());
                let start = centre(t);
                let segs = 3;
                let path: Vec<Vec3> = (0..=segs)
                    .map(|k| {
                        let s = k as f32 / segs as f32;
                        start + d * (len * s) - Vec3::Y * (r.droop * len * s * s * 0.5)
                    })
                    .collect();
                let r0 = (r.trunk_r * taper(t) * 0.45).max(0.02);
                g.tube(&path, &[r0, r0 * 0.6, r0 * 0.35, r0 * 0.15], if r0 > 0.06 { 4 } else { 3 });
                // Leaf cards along the outer part of the branch.
                let cards = r.cards.max(1);
                for c in 0..cards {
                    let s = if cards == 1 { 0.85 } else { 0.4 + 0.6 * c as f32 / (cards - 1) as f32 };
                    let k = ((s * segs as f32) as usize).min(segs - 1);
                    let f = s * segs as f32 - k as f32;
                    let p = path[k].lerp(path[k + 1], f);
                    let along = (path[k + 1] - path[k]).normalize_or(d);
                    let side = along.cross(Vec3::Y).normalize_or(Vec3::X);
                    match r.leaf {
                        Leaf::Conifer => {
                            // Sprays lying along the branch, a second tilted across for depth.
                            g.card(p, along, side, r.card, r.card * 0.9);
                            let tilted = (side * 0.5 + Vec3::Y * 0.85).normalize();
                            g.card(p, along, tilted, r.card * 0.9, r.card * 0.8);
                        }
                        _ => {
                            // Cards facing out of the crown, a second crossed.
                            let up = (along + Vec3::Y * 0.6).normalize();
                            let side2 = up.cross(side).normalize_or(Vec3::Z);
                            g.card(p, up, side, r.card, r.card);
                            g.card(p, up, side2, r.card * 0.9, r.card * 0.9);
                        }
                    }
                }
            }
        }
    }
    g
}

/// Earth's species beside MR's broadleaf trees, in this order: three conifers (spruce, pine, fir),
/// two palms (coconut, date), an acacia and two shrubs.
pub fn earth_species() -> Vec<Recipe> {
    let base = Recipe {
        crown: Crown::Cone,
        height: 22.0,
        trunk_r: 0.3,
        crown_base: 0.12,
        spread: 0.2,
        branches: 34,
        tilt: -0.15,
        droop: 0.35,
        card: 1.7,
        cards: 3,
        bend: 0.2,
        bark: 3,
        leaf: Leaf::Conifer,
        seed: 11,
    };
    vec![
        base.clone(),
        Recipe { crown: Crown::Column, height: 20.0, trunk_r: 0.28, crown_base: 0.55, spread: 0.22, branches: 22, tilt: 0.25, droop: 0.15, card: 1.9, bark: 6, seed: 12, ..base.clone() },
        Recipe { height: 26.0, trunk_r: 0.32, crown_base: 0.08, spread: 0.15, branches: 40, tilt: -0.05, droop: 0.25, card: 1.5, bark: 7, seed: 13, ..base.clone() },
        Recipe { crown: Crown::Palm, height: 21.0, trunk_r: 0.24, crown_base: 1.0, spread: 0.33, branches: 14, tilt: 0.55, droop: 0.9, card: 1.6, cards: 1, bend: 2.2, bark: 8, leaf: Leaf::Palm, seed: 14 },
        Recipe { crown: Crown::Palm, height: 13.0, trunk_r: 0.32, crown_base: 1.0, spread: 0.36, branches: 18, tilt: 0.7, droop: 0.6, card: 1.3, cards: 1, bend: 0.3, bark: 1, leaf: Leaf::Palm, seed: 15 },
        Recipe { crown: Crown::Umbrella, height: 8.0, trunk_r: 0.22, crown_base: 0.65, spread: 0.75, branches: 18, tilt: 0.2, droop: 0.05, card: 1.6, cards: 3, bend: 0.8, bark: 2, leaf: Leaf::Branch(6), seed: 16 },
        Recipe { crown: Crown::Shrub, height: 2.5, trunk_r: 0.05, crown_base: 0.0, spread: 0.55, branches: 26, tilt: 0.0, droop: 0.0, card: 1.0, cards: 1, bend: 0.0, bark: 4, leaf: Leaf::Branch(3), seed: 17 },
        Recipe { crown: Crown::Shrub, height: 4.0, trunk_r: 0.07, crown_base: 0.0, spread: 0.45, branches: 30, tilt: 0.0, droop: 0.0, card: 1.3, cards: 1, bend: 0.0, bark: 5, leaf: Leaf::Branch(0), seed: 18 },
    ]
}

/// Species a flora has (and Earth's set). The cull pass picks by these places (scatter.wesl
/// `earth_species`, `flora_species`; keep them in step): Earth's conifers 0-2, palms 3-4, acacia 5,
/// shrubs 6-7; a flora's forest trees 0-2, highland trees 3-4, dry land 5-6, shore 7.
pub const SPECIES: u32 = 8;

/// A planet's flora: 8 species from its `seed`, its surface `gravity` (m/s^2) and its air's
/// `pressure` (atm). Low gravity grows tall thin trees, high gravity squat thick ones; dense air
/// spreads the crowns (more lift for leaves), thin air keeps them tight.
pub fn flora(seed: u32, gravity: f32, pressure: f32) -> Vec<Recipe> {
    let mut rng = seeded(seed ^ 0x5eed_f10a);
    let g = (gravity / 9.81).clamp(0.05, 5.0);
    let tall = g.powf(-0.5).clamp(0.6, 1.9);
    let thick = g.powf(0.5).clamp(0.6, 1.7);
    let spread = pressure.max(0.05).powf(0.2).clamp(0.7, 1.5);
    let mut out = Vec::with_capacity(SPECIES as usize);
    for i in 0..SPECIES {
        let role = match i {
            0..=2 => 0,
            3..=4 => 1,
            5..=6 => 2,
            _ => 3,
        };
        let crown = match role {
            0 => [Crown::Broad, Crown::Broad, Crown::Cone, Crown::Column, Crown::Umbrella][(rng.next() * 5.0) as usize % 5],
            1 => [Crown::Cone, Crown::Column, Crown::Cone][(rng.next() * 3.0) as usize % 3],
            2 => [Crown::Shrub, Crown::Umbrella, Crown::Shrub][(rng.next() * 3.0) as usize % 3],
            _ => [Crown::Palm, Crown::Broad][(rng.next() * 2.0) as usize % 2],
        };
        let leaf = match crown {
            Crown::Cone | Crown::Column if rng.next() < 0.7 => Leaf::Conifer,
            Crown::Palm => Leaf::Palm,
            _ => Leaf::Branch((rng.next() * 8.0) as u8 % 8),
        };
        let height = match crown {
            Crown::Shrub => rng.range(1.5, 4.5),
            Crown::Umbrella => rng.range(6.0, 14.0),
            Crown::Palm => rng.range(8.0, 18.0),
            _ => rng.range(12.0, 30.0),
        } * tall;
        let r = Recipe {
            crown,
            height,
            trunk_r: height * rng.range(0.010, 0.018) * thick,
            crown_base: match crown {
                Crown::Cone => rng.range(0.05, 0.2),
                Crown::Column => rng.range(0.2, 0.5),
                Crown::Umbrella => rng.range(0.55, 0.75),
                Crown::Palm => 1.0,
                Crown::Shrub => 0.0,
                Crown::Broad => rng.range(0.25, 0.45),
            },
            spread: match crown {
                Crown::Cone => rng.range(0.13, 0.24),
                Crown::Column => rng.range(0.1, 0.18),
                Crown::Umbrella => rng.range(0.5, 0.85),
                Crown::Palm => rng.range(0.25, 0.4),
                Crown::Shrub => rng.range(0.35, 0.6),
                Crown::Broad => rng.range(0.25, 0.4),
            } * spread,
            branches: match crown {
                Crown::Palm => rng.range(10.0, 20.0) as u32,
                Crown::Shrub => rng.range(20.0, 32.0) as u32,
                Crown::Cone => rng.range(26.0, 40.0) as u32,
                _ => rng.range(16.0, 28.0) as u32,
            },
            tilt: match crown {
                Crown::Cone => rng.range(-0.3, 0.1),
                Crown::Palm => rng.range(0.4, 0.8),
                Crown::Umbrella => rng.range(0.05, 0.3),
                _ => rng.range(0.25, 0.75),
            },
            droop: match crown {
                Crown::Palm => rng.range(0.5, 1.0),
                Crown::Cone => rng.range(0.2, 0.5),
                _ => rng.range(0.0, 0.35),
            } / thick.sqrt(),
            card: match crown {
                Crown::Palm => rng.range(1.0, 1.8),
                _ => rng.range(1.1, 2.2),
            } * tall.sqrt(),
            cards: if matches!(crown, Crown::Palm | Crown::Shrub) { 1 } else { rng.range(2.0, 4.0) as u32 },
            bend: height * rng.range(0.0, 0.06) / thick,
            bark: (rng.next() * BARKS.len() as f32) as u8 % BARKS.len() as u8,
            leaf,
            seed: rng.0,
        };
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn species_grow_within_budget() {
        let mut all = earth_species();
        for s in [1u32, 7, 99, 12345] {
            all.extend(flora(s, 9.81 * (s % 3 + 1) as f32 * 0.6, 0.3 + (s % 5) as f32 * 0.4));
        }
        for r in &all {
            let g = grow(r);
            let tris = (g.bark.len() + g.leaves.len()) / 3;
            assert!((100..2600).contains(&tris), "{:?}: {tris} triangles", r.crown);
            assert!(!g.leaves.is_empty(), "{:?} has no leaves", r.crown);
            let top = g.positions.iter().map(|p| p[1]).fold(0.0f32, f32::max);
            assert!(top > r.height * 0.5 && top < r.height * 1.6, "{:?}: top {top} of {}", r.crown, r.height);
            assert!(g.positions.iter().flatten().all(|v| v.is_finite()));
            assert!(g.bark.iter().chain(&g.leaves).all(|&i| (i as usize) < g.positions.len()));
        }
    }

    #[test]
    fn gravity_shapes_the_flora() {
        let light: f32 = flora(5, 3.0, 1.0).iter().map(|r| r.height).sum();
        let heavy: f32 = flora(5, 25.0, 1.0).iter().map(|r| r.height).sum();
        assert!(light > heavy * 1.8, "{light} vs {heavy}");
    }
}
