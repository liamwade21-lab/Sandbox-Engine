//! Multicellular intelligent life (the vegetation and civilizations plan's step 5): which generated
//! worlds carry a civilization, and its level (1 primitive, 2 modern, 3 futuristic, 4 interstellar).
//!
//! It runs after SE's life generator ([`super::se_life`]) and draws only from its own hash of the
//! system's seed and the body's place in it, never from SE's streams: every generated system stays
//! exactly as it was, apart from the new tag (`Life { Type "Intelligent" Civilization N }`).
//!
//! - **Eligible:** organic multicellular life on land (a terrestrial biome), SE's complex stage
//!   reached (the value SE's generator drew), a mean surface temperature of 250-370 K, an air of
//!   0.1-30 atm. (Wider than Earth's: SE's land-life worlds are mostly warm and dense-aired, median
//!   339 K and 75 atm; with 260-310 K and 0.3-5 atm only 3 % of them qualified, too few for the
//!   decided 1 in 20.)
//! - **Emergence:** intelligence appears a lognormal time after complex life (median 0.5 Gyr: on
//!   Earth complex life came at about 4.0 Gyr and people at 4.6); the world must be older. Then a
//!   base chance (the "hard steps": Carter 1983, Snyder-Beattie et al. 2021), calibrated so that
//!   about 1 in 20 planets with multicellular land life carries a civilization.
//! - **Level:** from weights calibrated to the target mix (45 / 15 / 20 / 20 %) after the caps,
//!   shifted towards the later levels the longer ago intelligence appeared; then capped: an air with
//!   under 12 % oxygen allows level 1 at most (no open fire, no metallurgy); a surface gravity over
//!   2 g level 3 at most (chemical rockets hardly reach orbit from such worlds: Hippke 2018).
//!   SE's completed atmospheres hold no oxygen from life, so a land biosphere's own counts: it builds
//!   Earth's 21 % over the first half billion years of complex life.
//!
//! `cargo test -p se_core print_civ_stats -- --ignored --nocapture` prints the calibration.

use super::se_life::{LifeIn, LifeOut};

/// The chance, for an eligible world old enough for intelligence, that it arose (calibrated on
/// 20,000 generated systems, 2026-11-04: 1,672 planets with multicellular land life, 83 with a
/// civilization, 1 in 20.1; the levels then 45 / 15 / 19 / 20 %).
pub const BASE_CHANCE: f32 = 0.30;
/// The levels' weights before the time shift and the caps (calibrated so the final mix is 45 / 15
/// / 20 / 20 %).
pub const LEVEL_WEIGHTS: [f32; 4] = [0.641, 0.157, 0.133, 0.069];
/// How strongly the time since intelligence appeared moves the mix (per level, at full shift).
const SHIFT: f32 = 0.6;
const EMERGE_MEDIAN_GYR: f32 = 0.5;
const EMERGE_SIGMA: f32 = 0.8;
const O2_FIRE: f32 = 0.12;
const GRAVITY_ROCKETS: f32 = 2.0;
const SALT: u32 = 0xc1a1_2a7e;

/// The intelligence the planet drew: whether it emerged (and how long ago, Gyr) before the base
/// chance and the caps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Draw {
    pub emerged_ago: Option<f32>,
    pub chance: f32,
    pub level_u: f32,
}

fn hash(mut h: u32) -> u32 {
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

struct Draws(u32);

impl Draws {
    fn next(&mut self) -> f32 {
        self.0 = hash(self.0.wrapping_add(0x9e37_79b9));
        ((self.0 >> 8) as f32 + 0.5) / 16_777_216.0
    }
}

/// Organic multicellular life on land (what the decisions call land life with a chance of minds).
pub(super) fn land_life(l: &LifeOut) -> bool {
    !l.exotic && l.multicellular && l.biome.contains("Terrestrial")
}

/// Eligible for a civilization (before its own draws).
pub(super) fn eligible(b: &LifeIn, l: &LifeOut) -> bool {
    land_life(l) && l.complex_gyr < b.age_gyr && (250.0..=370.0).contains(&b.t_surf) && (0.1..=30.0).contains(&b.pressure_atm)
}

/// The oxygen share of the world's air for its civilization: the completed atmosphere's, or what a
/// land biosphere has built (up to Earth's 21 % over the first 0.5 Gyr of complex life).
pub(super) fn oxygen(b: &LifeIn, l: &LifeOut) -> f32 {
    b.o2.max(0.21 * ((b.age_gyr - l.complex_gyr) / 0.5).clamp(0.0, 1.0))
}

/// The world's own draws (from the system's `seed` and the body's place `index` in it).
pub(super) fn draw(seed: u32, index: usize, b: &LifeIn, l: &LifeOut) -> Draw {
    let mut d = Draws(hash(seed ^ SALT) ^ hash((index as u32).wrapping_mul(0x632b_e5ab) ^ SALT));
    let (u1, u2) = (d.next(), d.next());
    let gauss = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
    let t_int = l.complex_gyr + EMERGE_MEDIAN_GYR * (EMERGE_SIGMA * gauss).exp();
    let emerged_ago = (b.age_gyr > t_int).then_some(b.age_gyr - t_int);
    Draw { emerged_ago, chance: d.next(), level_u: d.next() }
}

/// The level from the world's draws and the weights (before the caps).
pub(super) fn pick_level(d: &Draw, weights: &[f32; 4]) -> u8 {
    let dt = d.emerged_ago.unwrap_or(0.0);
    let s = ((dt - 0.6) / 1.5).clamp(-1.0, 1.0);
    let w: Vec<f32> = (0..4).map(|k| weights[k] * (SHIFT * s * (k as f32 - 1.5)).exp()).collect();
    let total: f32 = w.iter().sum();
    let mut acc = 0.0;
    for (k, wk) in w.iter().enumerate() {
        acc += wk / total;
        if d.level_u < acc {
            return k as u8 + 1;
        }
    }
    4
}

/// The planet's caps on its level: oxygen for fire, gravity for rockets.
pub(super) fn cap(level: u8, b: &LifeIn, l: &LifeOut) -> u8 {
    let mut level = level;
    if oxygen(b, l) < O2_FIRE {
        level = level.min(1);
    }
    if b.gravity_g > GRAVITY_ROCKETS {
        level = level.min(3);
    }
    level
}

/// The civilization level of a world (0 none).
pub(super) fn level(seed: u32, index: usize, b: &LifeIn, l: &LifeOut) -> u8 {
    if !eligible(b, l) {
        return 0;
    }
    let d = draw(seed, index, b, l);
    if d.emerged_ago.is_none() || d.chance >= BASE_CHANCE {
        return 0;
    }
    cap(pick_level(&d, &LEVEL_WEIGHTS), b, l)
}

/// Gives the system's eligible worlds their civilizations (`lives` as [`super::se_life`]'s
/// generator returns them, per body in `bodies`' order).
pub(super) fn assign(seed: u32, bodies: &[LifeIn], lives: &mut [Vec<LifeOut>]) {
    for (i, (b, ls)) in bodies.iter().zip(lives.iter_mut()).enumerate() {
        for l in ls.iter_mut() {
            l.civilization = level(seed, i, b, l);
        }
    }
}

/// The level word the info panel shows.
pub fn level_name(level: u8) -> &'static str {
    match level {
        1 => "primitive",
        2 => "modern",
        3 => "futuristic",
        4 => "interstellar",
        _ => "",
    }
}

/// A civilization's look from orbit (the plan's culture seed): its lights' colour (linear, about
/// unit brightness) and how bright they are, its towns' colour by day (linear albedo), and how much
/// it likes the coast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Culture {
    pub light: [f32; 3],
    pub brightness: f32,
    pub urban: [f32; 3],
    pub coast: f32,
    /// How far its street grids turn from the cube face's axes: 0 all aligned (planned grids), 1 any
    /// angle (grown towns).
    pub grid_turn: f32,
}

/// The culture of a civilization of `level` with seed `seed` (a hash of the planet's name): primitive
/// peoples' fires, modern sodium or white lamps, futuristic and interstellar ones any hue (alien
/// tastes); thatch and earth, concrete, or a palette of its own by day.
pub fn culture(seed: u32, level: u8) -> Culture {
    let mut d = Draws(hash(seed ^ 0x0c17_7a2e));
    let (u1, u2, u3) = (d.next(), d.next(), d.next());
    const MODERN: [[f32; 3]; 3] = [[1.0, 0.62, 0.28], [1.0, 0.85, 0.65], [0.88, 0.94, 1.0]];
    const ALIEN: [[f32; 3]; 6] = [[1.0, 0.62, 0.28], [0.88, 0.94, 1.0], [0.55, 1.0, 0.55], [0.5, 0.9, 1.0], [0.8, 0.55, 1.0], [1.0, 0.5, 0.7]];
    let (light, brightness) = match level {
        1 => ([1.0, 0.55, 0.2], 0.12),
        2 => (MODERN[(u1 * 3.0) as usize % 3], 1.0),
        3 => (ALIEN[(u1 * 6.0) as usize % 6], 1.6),
        _ => (ALIEN[(u1 * 6.0) as usize % 6], 2.2),
    };
    let urban = match level {
        1 => [0.30, 0.25, 0.18],
        2 => [0.32, 0.31, 0.29],
        _ => {
            // Pale, dark or tinted: a hue of its own at a modest saturation.
            let v = 0.18 + 0.32 * u2;
            let h = u3 * std::f32::consts::TAU;
            let s = 0.12;
            [v * (1.0 + s * h.cos()), v * (1.0 + s * (h - 2.09).cos()), v * (1.0 + s * (h + 2.09).cos())]
        }
    };
    let coast = 0.5 + d.next();
    Culture { light, brightness, urban, coast, grid_turn: d.next() }
}

/// The culture seed of a planet: a hash of its name.
pub fn culture_seed(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// Where a body stands to a level 4 civilization's home world (the plan's step 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColonySite {
    /// One of the home world's moons.
    HomeMoon,
    /// Another planet of the system.
    Planet,
    /// A moon of another planet.
    OtherMoon,
}

/// Whether a level 4 civilization of culture seed `seed` has colonized the body called `name` (a solid
/// body of `radius_km` and `mass_earth`: no giants, at least 150 km across): the home world's moons 4 in
/// 5, the other planets 1 in 2, their moons 1 in 4, by the culture's hash.
pub fn colonized(seed: u32, name: &str, site: ColonySite, radius_km: f64, mass_earth: f64) -> bool {
    if !(150.0..12_000.0).contains(&radius_km) || mass_earth > 10.0 {
        return false;
    }
    let chance = match site {
        ColonySite::HomeMoon => 0.8,
        ColonySite::Planet => 0.5,
        ColonySite::OtherMoon => 0.25,
    };
    let h = culture_seed(name) ^ seed.wrapping_mul(0x9e37_79b9);
    st_hash([h as i32, 17, 4], seed)[0] < chance
}

/// A level 4 civilization's orbital station: round its home world (`moon` None) or one of its moons
/// (the index into the moons given), the orbit's radius in the body's radii, its tilt from the
/// equator and its node (rad), its phase (rad), its design (0 the ring station, 1 the truss station)
/// and its scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StationSpec {
    pub moon: Option<usize>,
    pub radius: f64,
    pub tilt: f64,
    pub node: f64,
    pub phase: f64,
    pub design: u8,
    pub scale: f64,
}

/// The stations of a level 4 civilization of culture seed `seed` (the plan: 3-12 round the home world,
/// big ones at its moons): round the home world 3 to 12 in low and middle orbits, and a big ring
/// station round each of its moons of `big_moon_km` radius or more (`moon_radii_km`).
pub fn stations(seed: u32, moon_radii_km: &[f64]) -> Vec<StationSpec> {
    const BIG_MOON_KM: f64 = 400.0;
    let n = 3 + (st_hash([seed as i32, 5, 9], seed)[0] * 10.0) as usize;
    let mut out = Vec::new();
    for i in 0..n.min(12) {
        let h = st_hash([i as i32, 11, 3], seed);
        let h2 = st_hash([i as i32, 12, 3], seed);
        out.push(StationSpec {
            moon: None,
            radius: 1.05 + 0.6 * h[0] as f64 * h[0] as f64,
            tilt: (h[1] as f64 - 0.5) * 1.2,
            node: h[2] as f64 * std::f64::consts::TAU,
            phase: h[3] as f64 * std::f64::consts::TAU,
            design: (h2[0] < 0.4) as u8,
            scale: 0.6 + 0.6 * h2[1] as f64,
        });
    }
    for (m, &r) in moon_radii_km.iter().enumerate() {
        if r >= BIG_MOON_KM {
            let h = st_hash([m as i32, 13, 3], seed);
            out.push(StationSpec { moon: Some(m), radius: 1.3 + 0.5 * h[0] as f64, tilt: (h[1] as f64 - 0.5) * 0.4, node: h[2] as f64 * std::f64::consts::TAU, phase: h[3] as f64 * std::f64::consts::TAU, design: 0, scale: 1.5 });
        }
    }
    out
}

/// settlement.wesl's `st_hash` on the CPU (the same u32 arithmetic): four numbers in 0..1 for cell
/// `c` and seed `seed`.
pub fn st_hash(c: [i32; 3], seed: u32) -> [f32; 4] {
    let mut h = (c[0] as u32).wrapping_mul(0x8da6_b343) ^ (c[1] as u32).wrapping_mul(0xd816_3841) ^ (c[2] as u32).wrapping_mul(0xcb1a_b31f) ^ seed.wrapping_mul(0x9e37_79b9);
    let mut o = [0.0; 4];
    for v in &mut o {
        h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
        h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
        h ^= h >> 16;
        *v = (h >> 8) as f32 * (1.0 / 16_777_216.0);
    }
    o
}

/// A settlement size of settlement.wesl `settlements` for `level`: (cell km, chance, radius km low,
/// high, seed offset). `kind` 0 villages, 1 towns, 2 cities, 3 megalopolises; `None` when the level
/// has none of that size.
pub fn settlement_size(level: u8, kind: u8) -> Option<(f64, f32, f32, f32, u32)> {
    let big = level >= 3;
    match kind {
        0 => Some((14.0, 0.45, 0.25, 0.9, 0)),
        1 if level >= 2 => Some((55.0, 0.9, if big { 2.5 } else { 1.5 }, if big { 10.0 } else { 7.0 }, 1)),
        2 if level >= 2 => Some((280.0, if big { 0.7 } else { 0.65 }, if big { 20.0 } else { 10.0 }, if big { 70.0 } else { 35.0 }, 2)),
        3 if level >= 4 => Some((420.0, 0.7, 45.0, 130.0, 3)),
        _ => None,
    }
}

/// The settlement of size `kind` (see [`settlement_size`]) nearest body-frame direction `dir` on a
/// body of `radius_m` whose civilization has `level` and culture seed `seed` (the ground's: the
/// seed fraction's 4 digits): its middle's direction and its radius (km). Only settlements whose
/// middle lies within half their radius of the surface (they reach the ground); within `search_km`.
/// The land there is not checked (it may be sea or mountains, where the ground draws no town).
pub fn nearest_settlement(dir: glam::DVec3, radius_m: f64, level: u8, seed: u32, kind: u8, search_km: f64) -> Option<(glam::DVec3, f64)> {
    let (cell, chance, r_lo, r_hi, off) = settlement_size(level, kind)?;
    let rk = radius_m / 1000.0;
    let p = dir.normalize() * rk;
    let n = (search_km / cell).ceil() as i32 + 1;
    let c0 = (p / cell).floor();
    let mut best: Option<(f64, glam::DVec3, f64)> = None;
    for dz in -n..=n {
        for dy in -n..=n {
            for dx in -n..=n {
                let c = [c0.x as i32 + dx, c0.y as i32 + dy, c0.z as i32 + dz];
                let h = st_hash(c, seed.wrapping_add(off));
                if h[3] >= chance {
                    continue;
                }
                let t = (h[3] * 7.31 + h[0] * 3.17).fract();
                let r = (r_lo * (r_hi / r_lo).powf(t)) as f64;
                let q = (glam::DVec3::new(c[0] as f64, c[1] as f64, c[2] as f64) + glam::DVec3::new(h[0] as f64, h[1] as f64, h[2] as f64)) * cell;
                if (q.length() - rk).abs() > 0.5 * r {
                    continue;
                }
                let d = q.normalize().dot(dir.normalize()).clamp(-1.0, 1.0).acos() * rk;
                if d <= search_km && best.is_none_or(|b| d < b.0) {
                    best = Some((d, q.normalize(), r));
                }
            }
        }
    }
    best.map(|(_, d, r)| (d, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level 4's colonies and stations: the shares the plan asks for, every count in range.
    #[test]
    fn colonies_and_stations() {
        let (mut home, mut planet, mut other) = (0, 0, 0);
        for i in 0..2000u32 {
            let seed = culture_seed(&format!("Home {i}"));
            home += colonized(seed, &format!("Moon {i}"), ColonySite::HomeMoon, 800.0, 0.01) as u32;
            planet += colonized(seed, &format!("Planet {i}"), ColonySite::Planet, 3000.0, 0.3) as u32;
            other += colonized(seed, &format!("Far moon {i}"), ColonySite::OtherMoon, 600.0, 0.005) as u32;
            assert!(!colonized(seed, "Giant", ColonySite::Planet, 60_000.0, 300.0));
            assert!(!colonized(seed, "Pebble", ColonySite::HomeMoon, 50.0, 1e-6));
            let s = stations(seed, &[1700.0, 100.0]);
            let round_home = s.iter().filter(|s| s.moon.is_none()).count();
            assert!((3..=12).contains(&round_home), "{round_home}");
            assert_eq!(s.iter().filter(|s| s.moon == Some(0)).count(), 1);
            assert!(s.iter().all(|s| s.moon != Some(1) && s.radius > 1.0));
        }
        let share = |n: u32| n as f64 / 2000.0;
        assert!((share(home) - 0.8).abs() < 0.05 && (share(planet) - 0.5).abs() < 0.05 && (share(other) - 0.25).abs() < 0.05, "{home} {planet} {other}");
    }

    #[test]
    fn cultures_suit_their_levels() {
        let mut hues = std::collections::HashSet::new();
        for i in 0..200u32 {
            let seed = culture_seed(&format!("Test planet {i}"));
            for level in 1..=4u8 {
                let c = culture(seed, level);
                assert!(c.light.iter().chain(&c.urban).all(|v| v.is_finite() && *v > 0.0 && *v <= 1.0));
                assert!((0.5..1.5).contains(&c.coast));
                // Fires are dim; the higher levels brighter.
                assert!(c.brightness == [0.12, 1.0, 1.6, 2.2][level as usize - 1]);
                if level == 3 {
                    hues.insert(c.light.map(|v| (v * 100.0) as u32));
                }
            }
            // The same name always gives the same culture.
            assert_eq!(culture(seed, 3), culture(culture_seed(&format!("Test planet {i}")), 3));
        }
        // Futuristic peoples' lights come in every hue of the palette.
        assert_eq!(hues.len(), 6);
    }
}
