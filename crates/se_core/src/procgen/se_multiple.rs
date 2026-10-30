//! SE's multiple-star systems for procedural stars (seSolarSystem::CreateProcedural,
//! FUN_1404f7c40, and the binary branch of FUN_1404f9210).
//!
//! - **Hierarchy**: the star splits into A and B with ProbBinaryStar (0.5; 0.95 for luminosity code
//!   10). A and B each split with ProbTripleStar (0.11), and each of their halves with
//!   ProbQuadrupleStar (0.22). That gives up to eight stars: A, B, AA, AB, BA, BB, then AAA ... BBB.
//!   A sub-split's separation is capped at 0.3 of the component's sphere of influence; a draw past
//!   the cap leaves the star whole. Stars of at most 0.045 solar masses or 9.33e-6 L_sun never split.
//! - **Companions**: a remnant companion with probability ProbCompanionWD + NS + BH (2.25%):
//!   white dwarf `tn(1, 0.5, 0.56, 1.44)`, neutron star `tn(2.22, 0.39, 1.44, 3)`, black hole
//!   `tn(8, 3, 3, 100)` solar masses. A star older than its lifetime is then reset to the mass
//!   whose lifetime is the system's age. Otherwise a brown dwarf with ProbBrownDwarf (0.025,
//!   x0.2 after each one) of `tn(0.03, 0.02, 0.013, 0.08)` solar masses. Otherwise the light
//!   splits: A gets `L(q M)` (SE's mass-luminosity law, `q = tn(0.75, 0.3, 0.01, 0.99)`), at
//!   least half, and B, a main-sequence star, the rest. The heavier star is A.
//! - **Orbit**: `log10(a/AU) = tn(log10(cap)/1.84, 2.5, -2, log10(cap))` (-3 for code 10),
//!   at least 2.5 (R_A + R_B). `e = tn(0.5, 0.5, 0, 0.89)`, zero below a periapsis of
//!   2.5 (R_A + R_B) and scaled by `(log10 a + 1)/0.7` up to 1. The plane is random for the
//!   barycentre, with the stars' orbits within +-2.5 degrees of it.
//! - **Planets**: every component star gets `rand % 10 + 5` slots. A component's sphere of
//!   influence is the periapsis separation over `sqrt(M_other/M/0.05) + 1`. The root barycentre
//!   gets circumbinary slots out to `max(10 apoapsis, tn(40, 8, 10, 1000) M)` AU, starting beyond
//!   3 apoapsis separations ([`se_layout::place`]).
//!
//! Not decoded: SE's class tables (FUN_1401d6a50 and the table at 0x140807d40, built at start-up)
//! and FUN_1405045a0's star physics. A component's temperature, radius and class come from the
//! port's main-sequence relations, and a remnant's temperature is the port's draw. The split
//! decisions draw from per-star streams of the port's: SE draws them from whatever stream
//! precedes them.

use super::se_layout::{self, ua, SeStar};
use super::tilt_normal;
use crate::star::{SpectralClass, StarKind};
use crate::surface_gen::SeRng;

const PROB_BINARY: f32 = 0.5;
const PROB_TRIPLE: f32 = 0.11;
const PROB_QUADRUPLE: f32 = 0.22;
const PROB_BROWN_DWARF: f32 = 0.025;
const PROB_COMPANION_WD: f32 = 0.01;
const PROB_COMPANION_NS: f32 = 0.005;
const PROB_COMPANION_BH: f32 = 0.0075;
const KM_PER_AU: f64 = 149_597_870.7;
const SOLAR_RADIUS_KM: f64 = 695_700.0;
const DEG: f64 = std::f64::consts::PI / 180.0;
const TAU: f64 = std::f64::consts::TAU;
const PI: f64 = std::f64::consts::PI;

/// One object of a multiple system: a star, or a barycentre with two children.
#[derive(Debug, Clone)]
pub(super) struct Comp {
    /// "" for the root, then "A", "B", "AA", ... (SE's name suffixes).
    pub suffix: String,
    pub parent: Option<usize>,
    pub children: Option<[usize; 2]>,
    /// SE's object index (its seeds).
    pub idx: u32,
    /// For a barycentre: its stars' total mass and light, its primary's class.
    pub star: SeStar,
    pub class: String,
    pub kind: StarKind,
    pub orbit: Option<BinOrbit>,
    pub slots: u32,
    pub soi_au: f64,
}

/// A component's orbit about its barycentre (ecliptic angles in degrees).
#[derive(Debug, Clone)]
pub(super) struct BinOrbit {
    pub a_au: f64,
    pub ecc: f64,
    pub inc_deg: f64,
    pub node_deg: f64,
    pub argp_deg: f64,
    pub ma_deg: f64,
    pub period_days: f64,
}

/// SE's main-sequence mass-luminosity law (FUN_1401dd880), L_sun from M_sun.
pub(super) fn lum_of_mass(m: f64) -> f64 {
    if m > 55.0 {
        m * 32_000.0
    } else if m > 2.0 {
        m.powf(3.5) * 1.4 + (55.0 - m) * 0.003_034_115
    } else if m > 0.85 {
        m.powi(4)
    } else if m > 0.2 {
        let m3 = m * m * m;
        m.powf(m3 * 232.4 - m3 * m * 141.7 - m * m * 129.1 + m * 33.29 + 0.215)
    } else if m > 0.1 {
        m.powf(2.3) * 0.23
    } else {
        m.max(0.003).powf(6.030_864) * 1237.633
    }
}

/// The mass with [`lum_of_mass`] `l` (bisection; the law rises monotonically).
fn mass_of_lum(l: f64) -> f64 {
    let (mut lo, mut hi) = (0.003f64, 300.0f64);
    for _ in 0..60 {
        let mid = (lo * hi).sqrt();
        if lum_of_mass(mid) < l {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo * hi).sqrt()
}

/// Bolometric luminosity of the port's main sequence at `t` K.
fn ms_lum(t: f64) -> f64 {
    crate::star::lum_from_abs_mag(crate::star::main_sequence_abs_mag(t)) * 10f64.powf(-0.4 * crate::star::bolometric_correction(t))
}

/// A main-sequence star (or brown dwarf) of light `l`: temperature, radius (km), class, from the
/// port's relations (SE's class tables are not decoded).
fn ms_star(l: f64) -> (f64, f64, String, StarKind) {
    let l = l.max(1e-12);
    let t = if l < ms_lum(2300.0) {
        // Below the main sequence: a brown dwarf about a tenth of a solar radius across.
        5772.0 * (l / 0.01).powf(0.25)
    } else {
        let (mut lo, mut hi) = (2300.0f64, 60_000.0f64);
        for _ in 0..50 {
            let mid = (lo * hi).sqrt();
            if ms_lum(mid) < l {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo * hi).sqrt()
    };
    let radius = crate::star::radius_from_lum_teff(l, t) * SOLAR_RADIUS_KM;
    let (class, kind) = if t >= 2400.0 {
        (format!("{}V", crate::star_octree::ms_class_name(t)), StarKind::Normal)
    } else if t >= 1300.0 {
        (format!("L{:.0}", ((2400.0 - t) / 110.0).clamp(0.0, 9.0).floor()), StarKind::BrownDwarf)
    } else if t >= 550.0 {
        (format!("T{:.0}", ((1300.0 - t) / 75.0).clamp(0.0, 9.0).floor()), StarKind::BrownDwarf)
    } else {
        ("Y0".to_string(), StarKind::BrownDwarf)
    };
    (t, radius, class, kind)
}

fn class_bits(class: &str) -> (u32, u32) {
    let (l, t, _) = crate::climate::star_class_bits(&SpectralClass::parse(class));
    (l, t)
}

/// A star of `class`, light `l` and mass `m` sharing the root's chemistry.
fn star_of(root: &SeStar, class: &str, t: f64, radius_km: f64, l: f64, m: f64) -> SeStar {
    let (lum_code, letter) = class_bits(class);
    SeStar { mass_sol: m, radius_km, teff: t, lum_sol: l, lum_code, letter, c_to_o: root.c_to_o, met: root.met }
}

/// A remnant of mass `m`: 0 white dwarf, 1 neutron star, 2 black hole. Temperatures are the
/// port's (SE's FUN_1401d5fe0 is not decoded).
fn remnant(root: &SeStar, kind: u8, m: f64, u: f64) -> (SeStar, String, StarKind) {
    let (t, radius, class, k) = match kind {
        0 => {
            let t = 5000.0 + u * u * 25_000.0;
            (t, 0.0126 * m.powf(-1.0 / 3.0) * SOLAR_RADIUS_KM, format!("DA{:.0}", (50_400.0 / t).clamp(1.0, 9.0)), StarKind::WhiteDwarf)
        }
        1 => (600_000.0, 12.0, "Q".to_string(), StarKind::NeutronStar),
        _ => (0.0, 2.953 * m, "X".to_string(), StarKind::BlackHole),
    };
    let l = if t > 0.0 { (radius / SOLAR_RADIUS_KM).powi(2) * (t / 5772.0).powi(4) } else { 0.0 };
    (star_of(root, &class, t, radius, l, m), class, k)
}

/// The root of a procedural system as the engine knows it.
pub(super) struct Root {
    pub star: SeStar,
    pub class: String,
    pub kind: StarKind,
    pub age_gyr: f64,
}

/// SE's multiple system for `root`, or `None` when it stays a single star.
pub(super) fn build(root: &Root, seed: u32) -> Option<Vec<Comp>> {
    let mut comps = vec![Comp {
        suffix: String::new(),
        parent: None,
        children: None,
        idx: 0,
        star: root.star.clone(),
        class: root.class.clone(),
        kind: root.kind,
        orbit: None,
        slots: 0,
        soi_au: 1000.0,
    }];
    let mut count = 1u32;
    let mut p_bd = PROB_BROWN_DWARF;
    if !split(&mut comps, 0, PROB_BINARY, 1000.0, &mut count, &mut p_bd, seed, root.age_gyr) {
        return None;
    }
    let [a, b] = comps[0].children.unwrap();
    for (first, cap_of) in [(a, a), (b, b)] {
        let cap = comps[cap_of].soi_au * 0.3;
        if split(&mut comps, first, PROB_TRIPLE, cap, &mut count, &mut p_bd, seed, root.age_gyr) {
            let [x, y] = comps[first].children.unwrap();
            for c in [x, y] {
                let cap = comps[c].soi_au * 0.3;
                split(&mut comps, c, PROB_QUADRUPLE, cap, &mut count, &mut p_bd, seed, root.age_gyr);
            }
        }
    }
    Some(comps)
}

/// The binary branch of FUN_1404f9210 for component `i`: true when it became a barycentre.
#[allow(clippy::too_many_arguments)]
fn split(comps: &mut Vec<Comp>, i: usize, p: f32, cap: f64, count: &mut u32, p_bd: &mut f32, seed: u32, age: f64) -> bool {
    let me = comps[i].clone();
    let (m, l) = (me.star.mass_sol, me.star.lum_sol);
    let wr = me.star.lum_code == 10;
    let p = if wr && p != 0.0 { 0.95 } else { p };
    // The port's decision stream (SE draws from whatever precedes the call).
    let mut dr = SeRng::new(seed ^ me.idx.wrapping_mul(0x9e37_79b9).wrapping_add(0x7f4a_7c15), seed.wrapping_mul(0x85eb_ca6b) ^ me.idx);
    if dr.uniform(1.0) >= p || m <= 0.045 || l <= 9.33e-6 {
        return false;
    }
    let (ia, ib) = (*count, *count + 1);
    let p3 = me.idx;
    let mut r = SeRng::new(
        seed.wrapping_mul(0x86).wrapping_add(p3.wrapping_mul(0x1616)).wrapping_add(0x10b6),
        seed.wrapping_mul(0x144).wrapping_add(p3.wrapping_mul(0x87b)).wrapping_add(0x1c50),
    );
    let root = &me.star;
    let u = r.uniform(1.0);
    let main_split = |r: &mut SeRng, mean: f32, sigma: f32, lo: f32, hi: f32| {
        let q = r.tnormal(mean, sigma, lo, hi) as f64;
        let mut la = lum_of_mass(q * m);
        if l < la {
            la = r.tnormal(0.9, 0.03, 0.7, 0.9999) as f64 * l;
        }
        if la < l * 0.5 {
            la = l - la;
        }
        (q, la)
    };
    // The original star with light `la` (its class kept; a dwarf's mass from its light).
    let keep = |la: f64, q: f64| {
        let t = me.star.teff;
        let radius = me.star.radius_km * (la / l.max(1e-30)).sqrt();
        let mass = if (6..=8).contains(&me.star.lum_code) { mass_of_lum(la) } else { m * q };
        (star_of(root, &me.class, t, radius, la, mass), me.class.clone(), me.kind)
    };
    let main_seq = |lb: f64| {
        let (t, radius, class, kind) = ms_star(lb);
        (star_of(root, &class, t, radius, lb, mass_of_lum(lb)), class, kind)
    };
    let mut sa;
    let mut sb;
    if wr {
        let (q, la) = main_split(&mut r, 0.65, 0.07, 0.5, 0.8);
        sa = keep(la, q);
        sb = main_seq(l - la);
    } else if u >= PROB_COMPANION_WD + PROB_COMPANION_NS + PROB_COMPANION_BH {
        if *p_bd <= r.uniform(1.0) {
            let (q, la) = main_split(&mut r, 0.75, 0.3, 0.01, 0.99);
            sa = keep(la, q);
            sb = main_seq(l - la);
        } else {
            let mb = r.tnormal(0.03, 0.02, 0.013, 0.08) as f64;
            sa = (me.star.clone(), me.class.clone(), me.kind);
            sb = main_seq(lum_of_mass(mb));
            sb.0.mass_sol = mb;
        }
    } else {
        let (kind, mean, sigma, lo, hi) = if u < PROB_COMPANION_BH {
            (2u8, 8.0, 3.0, 3.0, 100.0)
        } else if u < PROB_COMPANION_BH + PROB_COMPANION_NS {
            (1, 2.22, 0.39, 1.44, 3.0)
        } else {
            (0, 1.0, 0.5, 0.56, 1.44)
        };
        let mr = r.tnormal(mean, sigma, lo, hi) as f64;
        let rem = remnant(root, kind, mr, (u as f64 * 97.0).fract());
        if mr <= m {
            sb = rem;
            sa = (me.star.clone(), me.class.clone(), me.kind);
            // A primary older than its lifetime is reset to the mass whose lifetime is the age.
            if crate::climate::star_lifetime_gyr(sa.0.mass_sol as f32, sa.0.lum_code) < age as f32 {
                let mut ma = (age / 10.0).powf(-0.266_666_68);
                if !(0.012..=180.0).contains(&ma) {
                    ma = 0.2;
                }
                sa = main_seq(lum_of_mass(ma));
                sa.0.mass_sol = ma;
            }
        } else {
            sa = rem;
            sb = (me.star.clone(), me.class.clone(), me.kind);
        }
    }
    if sb.0.brown_dwarf() {
        *p_bd *= 0.2;
    }
    if sb.0.mass_sol > sa.0.mass_sol {
        std::mem::swap(&mut sa, &mut sb);
    }
    let x = se_layout_tn_d(&mut r, cap.log10() / 1.84, 2.5, -2.0, cap.log10()) + if wr { -3.0 } else { 0.0 };
    let min_sep = (sa.0.radius_km + sb.0.radius_km) * 2.5 / KM_PER_AU;
    let sep = min_sep.max(10f64.powf(x));
    if sep >= cap {
        return false;
    }
    let mt = sa.0.mass_sol + sb.0.mass_sol;
    if me.parent.is_none() {
        comps[i].slots = r.int(10) + 5;
    }
    let q = sa.0.mass_sol / mt;
    let period_days = (sep * sep * sep * 1.000_003_003_489_382_5 / mt).sqrt() * 365.242_189_85;
    let mut ecc = se_layout_tn_d(&mut r, 0.5, 0.5, 0.0, 0.89);
    let ma = ua(&mut r) * TAU - PI;
    let argp = ua(&mut r) * TAU - PI;
    let frame_node = ua(&mut r) * TAU - PI;
    let frame_inc = ua(&mut r) * TAU - PI;
    let inc = (2.5 - ua(&mut r) * 5.0) * DEG;
    // The stars' spin tilts.
    for _ in 0..4 {
        ua(&mut r);
    }
    let mut k = (sep.log10() + 1.0) / 0.7;
    if sep * (1.0 - ecc) < min_sep {
        k = 0.0;
    }
    ecc *= k.clamp(0.0, 1.0);
    let slots_a = r.int(10) + 5;
    let slots_b = r.int(10) + 5;
    // The orbit plane: the barycentre's random frame tilted by the orbits' inclination.
    let (oi, on) = tilt_normal(frame_inc.abs() / DEG, frame_node.rem_euclid(TAU) / DEG, inc / DEG, 0.0);
    let orbit = |a_au: f64, argp: f64| BinOrbit {
        a_au,
        ecc,
        inc_deg: oi,
        node_deg: on,
        argp_deg: (argp / DEG).rem_euclid(360.0),
        ma_deg: (ma / DEG).rem_euclid(360.0),
        period_days,
    };
    let peri = sep * (1.0 - ecc);
    let soi_a = peri / ((sb.0.mass_sol / sa.0.mass_sol / 0.05).sqrt() + 1.0);
    let soi_b = peri / ((sa.0.mass_sol / sb.0.mass_sol / 0.05).sqrt() + 1.0);
    // The barycentre's own reach (as a single star's, at least ten apoapsis separations).
    let soi_bary = ((sep * (1.0 + ecc) * 10.0).max(r.tnormal(40.0, 8.0, 10.0, 1000.0) as f64 * mt)).min(cap);
    let mut bary = me.star.clone();
    bary.mass_sol = mt;
    bary.lum_sol = sa.0.lum_sol + sb.0.lum_sol;
    bary.lum_code = sa.0.lum_code;
    bary.letter = sa.0.letter;
    bary.radius_km = sa.0.radius_km;
    bary.teff = sa.0.teff;
    let c = &mut comps[i];
    c.star = bary;
    c.class = sa.1.clone();
    c.soi_au = soi_bary;
    let (na, nb) = (comps.len(), comps.len() + 1);
    comps[i].children = Some([na, nb]);
    let suffix = |s: &str| if me.suffix.is_empty() { s.to_string() } else { format!("{}{}", me.suffix, s) };
    comps.push(Comp {
        suffix: suffix("A"),
        parent: Some(i),
        children: None,
        idx: ia,
        star: sa.0,
        class: sa.1,
        kind: sa.2,
        orbit: Some(orbit((1.0 - q) * sep, argp)),
        slots: slots_a,
        soi_au: soi_a,
    });
    comps.push(Comp {
        suffix: suffix("B"),
        parent: Some(i),
        children: None,
        idx: ib,
        star: sb.0,
        class: sb.1,
        kind: sb.2,
        orbit: Some(orbit(q * sep, argp + PI)),
        slots: slots_b,
        soi_au: soi_b,
    });
    *count += 2;
    true
}

/// SE's truncated normal in doubles (FUN_140441b80), on stream B like the float one.
fn se_layout_tn_d(r: &mut SeRng, mean: f64, sigma: f64, lo: f64, hi: f64) -> f64 {
    r.tnormal(mean as f32, sigma as f32, lo as f32, hi as f32) as f64
}

/// Inner edge (AU) of planets around component `i` (FUN_14050a890): a star's own, a barycentre's
/// the largest of its children's and 3 apoapsis separations plus 1.05 of their radii.
pub(super) fn inner_au(comps: &[Comp], i: usize, hot: bool) -> f64 {
    let c = &comps[i];
    let Some([a, b]) = c.children else {
        return se_layout::inner_limit_au(&c.star, hot);
    };
    let apo = |j: usize| comps[j].orbit.as_ref().map_or(0.0, |o| o.a_au * (1.0 + o.ecc));
    let edge = (apo(a) + apo(b)) * 3.0 + (comps[a].star.radius_km + comps[b].star.radius_km) * 1.05 / KM_PER_AU;
    inner_au(comps, a, hot).max(inner_au(comps, b, hot)).max(edge)
}
