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
