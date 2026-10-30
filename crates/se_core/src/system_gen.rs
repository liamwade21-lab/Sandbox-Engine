//! Steps of SE's planetary-system creation (FUN_140510f40 -> FUN_1404f62e0) that fill in what a
//! catalog leaves out: the system's random seed, the inclination and ascending node of planets
//! whose `Orbit` block omits them (FUN_14050d340) and of moons (FUN_140502eb0), and the mass of
//! planets known only by `Msini` (FUN_1404ca460).
//!
//! The angles are drawn per star record from the system seed: a mean from the record's children
//! whose inclination is known (or a uniform draw on stream A when none is), then a truncated
//! normal around it per planet on stream B. Proxima d and b, next to Proxima c's
//! `Inclination 133`, come out at 131.98 and 132.10 deg, so Msini / sin i gives SE's 0.35017 and
//! 1.4435 Earth masses.

use crate::catalog::{BodyTag, Catalog, ObjectRef};
use crate::sc::Block;
use crate::units::EARTH_MASS_KG;
use std::f64::consts::{FRAC_PI_2, PI, SQRT_2};

const DEG: f64 = PI / 180.0;
/// 80 Jupiter masses in Earth masses (an f32 in SE), the most a planet may weigh.
const MAX_PLANET_MASS: f64 = 25_426.271_484_375;
const INV_2_32: f64 = 1.0 / 4_294_967_296.0;
/// Light-years to parsecs as SE's star loader converts `DistLy`.
const LY_TO_PC: f64 = 0.306_601_128_784_947_5;

fn lcg(s: u32) -> u32 {
    s.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f)
}

/// A catalog star's position as SE's star loader computes it (FUN_140476e90): RA and Dec to
/// radians in double, then the trigonometry and products in f32 (universe frame, parsecs, Sun at
/// the origin). `None` without RA, Dec and a distance.
fn star_position_f32(b: &Block) -> Option<[f32; 3]> {
    let ra = (b.sexagesimal("RA")? * (PI / 12.0)) as f32;
    let dec = (b.sexagesimal("Dec")? * DEG) as f32;
    // Later keys win, as in the loader: Dist, then DistLy, then Parallax (mas).
    let dist = b
        .f64("Parallax")
        .map(|p| 1000.0 / p)
        .or_else(|| b.f64("DistLy").map(|d| d * LY_TO_PC))
        .or_else(|| b.f64("Dist"))? as f32;
    let sin = |a: f32| (a as f64).sin() as f32;
    let cos = |a: f32| (a as f64).cos() as f32;
    let cd = cos(dec);
    Some([sin(ra) * dist * cd, sin(dec) * dist, cos(ra) * dist * cd])
}

/// SE's seed for the planetary system around a catalog star or barycentre (FUN_140510f40,
/// logged as "Creating planetary system of star ..., seed = "): a hash of the position the star
/// loader stores, read in units of 2^-32 pc. The Solar System, at the origin, gets 0. Both random
/// streams start from it.
/// SE's age draw for a system without a catalog `Age` (FUN_1404f62e0): stream A's first value,
/// before the angle step draws.
pub fn age_draw(root: &Block) -> u32 {
    lcg(system_seed(root))
}

pub fn system_seed(block: &Block) -> u32 {
    let Some([x, y, z]) = star_position_f32(block) else { return 0 };
    let c = |v: f32| (v as f64 * 4_294_967_296.0).floor() as i64 as u32;
    c(x).wrapping_mul(0x28842)
        .wrapping_add(c(z).wrapping_mul(0x2309fb_u32.wrapping_neg()))
        .wrapping_add(c(y).wrapping_mul(0xe35ad))
}

/// SE's two random streams (thread-local; A at +0x24, B at +0x20).
#[derive(Debug, Clone, Copy)]
pub struct Streams {
    pub a: u32,
    pub b: u32,
}

impl Streams {
    fn uniform_a(&mut self) -> f64 {
        self.a = lcg(self.a);
        self.a as f64 * INV_2_32
    }
}

/// Inverse of the standard normal distribution (Cephes `ndtri`, FUN_140444dc0).
fn ndtri(y0: f64) -> f64 {
    const P0: [f64; 5] = [-5.996_335_010_141_079e1, 9.800_107_541_859_997e1, -5.667_628_574_690_703e1, 1.393_126_093_872_796_8e1, -1.239_165_838_673_812_6];
    const Q0: [f64; 8] = [1.954_488_583_381_417_6, 4.676_279_128_988_815, 8.636_024_213_908_906e1, -2.254_626_878_541_193_7e2, 2.002_602_123_800_606_6e2, -8.203_722_561_683_334e1, 1.590_562_251_262_117e1, -1.183_316_211_213_300_1];
    const P1: [f64; 9] = [4.055_448_923_059_624, 3.152_510_945_998_938_6e1, 5.716_281_922_464_213e1, 4.408_050_738_932_008e1, 1.468_495_619_288_580_2e1, 2.186_633_068_507_902_7, -1.402_560_791_713_545e-1, -3.504_246_268_278_482e-2, -8.574_567_851_546_854e-4];
    const Q1: [f64; 8] = [1.577_998_832_564_667_5e1, 4.539_076_351_288_792e1, 4.131_720_382_546_720_3e1, 1.504_253_856_929_075e1, 2.504_649_462_083_094, -1.421_829_228_547_877_9e-1, -3.808_064_076_915_783e-2, -9.332_594_808_954_574e-4];
    const P2: [f64; 9] = [3.237_748_917_769_460_4, 6.915_228_890_689_842, 3.938_810_252_924_744_4, 1.333_034_608_158_075_5, 2.014_853_895_491_790_8e-1, 1.237_166_348_178_200_2e-2, 3.015_815_535_082_354e-4, 2.658_069_746_867_375_5e-6, 6.239_745_391_849_833e-9];
    const Q2: [f64; 8] = [6.024_270_393_647_42, 3.679_835_638_561_608_6, 1.377_020_994_890_813_3, 2.162_369_935_944_966_3e-1, 1.342_040_060_885_431_9e-2, 3.280_144_646_821_277_4e-4, 2.892_478_647_453_806_8e-6, 6.790_194_080_099_813e-9];
    let polevl = |x: f64, c: &[f64]| c.iter().fold(0.0, |r, k| r * x + k);
    let p1evl = |x: f64, c: &[f64]| c.iter().fold(1.0, |r, k| r * x + k);
    const EXP_M2: f64 = 0.135_335_283_236_612_7;
    let (mut y, mut negate) = (y0, true);
    if y > 1.0 - EXP_M2 {
        y = 1.0 - y;
        negate = false;
    }
    if y > EXP_M2 {
        y -= 0.5;
        let y2 = y * y;
        return (y + y * (y2 * polevl(y2, &P0) / p1evl(y2, &Q0))) * 2.506_628_274_631_000_5;
    }
    let x = (-2.0 * y.ln()).sqrt();
    let x0 = x - x.ln() / x;
    let z = 1.0 / x;
    let x1 = if x < 8.0 { z * polevl(z, &P1) / p1evl(z, &Q1) } else { z * polevl(z, &P2) / p1evl(z, &Q2) };
    if negate { x1 - x0 } else { x0 - x1 }
}

/// Normal draw truncated to [lo, hi] on stream B (FUN_140441b80). With lo above hi it falls on
/// the far side of `lo`, as SE's inverse-CDF form does.
fn truncated_normal(s: &mut Streams, mean: f64, sigma: f64, lo: f64, hi: f64) -> f64 {
    s.b = lcg(s.b);
    let u = s.b as f64 * INV_2_32;
    let w = SQRT_2 * sigma;
    let (ea, eb) = (libm::erf((lo - mean) / w), libm::erf((hi - mean) / w));
    if eb - ea == 0.0 {
        return (hi - lo) * u + lo;
    }
    ndtri(((eb - ea) * u + ea + 1.0) * 0.5) / SQRT_2 * w + mean
}

/// A child of a star record as FUN_14050d340 sees it.
#[derive(Debug, Clone, Copy)]
pub struct Child {
    /// SE object type: 0 barycentre, 3 star, 4 planet, 5 moon, 6 dwarf planet, ...
    pub se_type: i32,
    /// `None` without an `Orbit` block; otherwise its inclination and ascending node (radians),
    /// each `None` where the block omits it.
    pub orbit: Option<(Option<f64>, Option<f64>)>,
    pub msini: Option<f32>,
}

/// The inclination and node SE draws for the children of one star record (FUN_14050d340), in
/// the order SE links them; `None` where the catalog gives the angle or the child is not a
/// planet or dwarf planet.
pub fn generate_angles(s: &mut Streams, kids: &[Child]) -> Vec<(Option<f64>, Option<f64>)> {
    let counted = |k: &&Child| k.se_type < 4 || k.se_type == 4 || k.se_type == 6;
    let mut max_msini = 0.0f64;
    let (mut n, mut sum_i, mut sum_n, mut min_i, mut min_n) = (0usize, 0.0, 0.0, 1e38f64, 1e38f64);
    for k in kids.iter().filter(counted) {
        if k.se_type >= 4 {
            if let Some(m) = k.msini {
                max_msini = max_msini.max(m as f64);
            }
        }
        // Children with a known inclination set both means (the node one too, even where the
        // node is not given).
        if let Some((Some(i), node)) = k.orbit {
            let node = node.unwrap_or(0.0);
            n += 1;
            sum_i += i;
            sum_n += node;
            min_i = min_i.min(i);
            min_n = min_n.min(node);
        }
    }
    let lo_unknown = || (max_msini / MAX_PLANET_MASS).asin() + 3.0 * DEG;
    let spread = |mean: f64, vals: &mut dyn Iterator<Item = f64>, min: f64| {
        let var = vals.map(|v| (mean - v) * (mean - v)).sum::<f64>() / n as f64;
        if var == 0.0 { (DEG, min + 3.0 * DEG) } else { (var, min) }
    };
    let (mean_i, sig_i, lo_i) = if n == 0 {
        let lo = lo_unknown();
        ((FRAC_PI_2 - lo) * s.uniform_a() + lo, DEG, lo)
    } else {
        let mean = sum_i / n as f64;
        let (sig, lo) = spread(mean, &mut kids.iter().filter(counted).filter_map(|k| k.orbit.and_then(|o| o.0)), min_i);
        (mean, sig, lo)
    };
    let (mean_n, sig_n, lo_n) = if n == 0 {
        let lo = lo_unknown();
        ((PI - lo) * s.uniform_a() + lo, DEG, lo)
    } else {
        let mean = sum_n / n as f64;
        let (sig, lo) = spread(mean, &mut kids.iter().filter(counted).filter_map(|k| k.orbit.and_then(|o| o.1)), min_n);
        (mean, sig, lo)
    };
    kids.iter()
        .map(|k| match k.orbit {
            Some((incl, node)) if k.se_type == 4 || k.se_type == 6 => (
                incl.is_none().then(|| truncated_normal(s, mean_i, sig_i, lo_i, FRAC_PI_2)),
                node.is_none().then(|| truncated_normal(s, mean_n, sig_n, lo_n, PI)),
            ),
            _ => (None, None),
        })
        .collect()
}

/// Mass in Earth masses SE derives from `Msini` (FUN_1404ca460, `PlanetMsiniResolve 0`):
/// `Msini / sin i` with the orbit's inclination, at most 80 Jupiter masses.
pub fn msini_mass(msini: f32, inclination: f64) -> f64 {
    let s = inclination.sin();
    if s * MAX_PLANET_MASS < msini as f64 { MAX_PLANET_MASS } else { (msini as f64 / s) as f32 as f64 }
}

/// SE's object type of a catalog record and the rank it is linked among its siblings by
/// (table at 0x14067a748; ties keep catalog order).
fn se_type(cat: &Catalog, r: ObjectRef) -> Option<i32> {
    Some(match r {
        ObjectRef::Star(i) if cat.stars[i as usize].barycenter => 0,
        ObjectRef::Star(_) => 3,
        ObjectRef::Body(i) => match cat.bodies[i as usize].tag {
            BodyTag::Planet => 4,
            BodyTag::Moon => 5,
            BodyTag::DwarfPlanet => 6,
            BodyTag::DwarfMoon => 7,
            BodyTag::Asteroid => 8,
            BodyTag::Comet => 9,
            BodyTag::Spacecraft => 11,
        },
        _ => return None,
    })
}

fn link_rank(t: i32) -> i32 {
    const RANK: [i32; 14] = [1, 2, 7, 0, 3, 5, 4, 6, 8, 9, 10, 11, 12, 13];
    RANK.get(t as usize).copied().unwrap_or(t)
}

fn block_of(cat: &Catalog, r: ObjectRef) -> &Block {
    match r {
        ObjectRef::Star(i) => &cat.stars[i as usize].block,
        ObjectRef::Body(i) => &cat.bodies[i as usize].block,
        _ => unreachable!("only stars and bodies are linked"),
    }
}

/// Orbit angles SE draws for a catalog moon whose `Orbit` block omits them (FUN_140502eb0, the
/// first draws of the moon's completion): on stream A from the moon's own seed, the inclination
/// uniform in [-1.5, 1.5) deg, then the node uniform in [0, 360) deg. Radians; `None` where the
/// catalog gives the angle.
pub fn moon_angles(seed_a: u32, given: (Option<f64>, Option<f64>)) -> (Option<f64>, Option<f64>) {
    let mut s = Streams { a: seed_a, b: 0 };
    let incl = given.0.is_none().then(|| {
        s.a = lcg(s.a);
        (s.a as f64 * 3.0 * INV_2_32 - 1.5) * DEG
    });
    let node = given.1.is_none().then(|| {
        s.a = lcg(s.a);
        s.a as f64 * (2.0 * PI) * INV_2_32
    });
    (incl, node)
}

/// The orbit angles (radians) a record's `Orbit` (or `BinaryOrbit`) block gives, as
/// [`Child::orbit`]; `None` without one.
pub fn given_angles(b: &Block) -> Option<(Option<f64>, Option<f64>)> {
    let o = b.sub("Orbit").or_else(|| b.sub("BinaryOrbit"))?;
    let node = o.f64("AscendingNode").or_else(|| o.f64("PositionAngle"));
    Some((o.f64("Inclination").map(|v| v * DEG), node.map(|v| v * DEG)))
}

/// Children of a star record in SE's link order.
fn linked_children(cat: &Catalog, star: u32) -> Vec<(ObjectRef, i32)> {
    let mut kids: Vec<(ObjectRef, i32)> = cat
        .children_of(cat.display_name(ObjectRef::Star(star)))
        .iter()
        .filter_map(|&r| Some((r, se_type(cat, r)?)))
        .collect();
    kids.sort_by_key(|&(_, t)| link_rank(t));
    kids
}

/// Fills in the orbit inclination and node SE generates for catalog planets, dwarf planets and
/// moons whose `Orbit` block omits them, then gives planets known only by `Msini` SE's mass.
pub fn complete_catalog_orbits(cat: &mut Catalog) {
    let missing = |b: &Block| given_angles(b).is_some_and(|(i, n)| i.is_none() || n.is_none());
    let mut roots: Vec<u32> = (0..cat.bodies.len())
        .filter(|&i| matches!(cat.bodies[i].tag, BodyTag::Planet | BodyTag::DwarfPlanet) && missing(&cat.bodies[i].block))
        .filter(|&i| matches!(cat.find(&cat.bodies[i].parent), Some(ObjectRef::Star(_))))
        .filter_map(|i| cat.system_root(ObjectRef::Body(i as u32)))
        .collect();
    roots.sort_unstable();
    roots.dedup();
    let mut generated = Vec::new();
    for root in roots {
        let seed = system_seed(&cat.stars[root as usize].block);
        let mut s = Streams { a: seed, b: seed };
        // Star records in SE's record order: the root, then its tree depth first with children
        // in link order (FUN_1404f62e0 lists the system through FUN_1404fab30, stars first).
        fn depth_first(cat: &Catalog, star: u32, out: &mut Vec<u32>) {
            out.push(star);
            for (r, _) in linked_children(cat, star) {
                if let ObjectRef::Star(j) = r {
                    depth_first(cat, j, out);
                }
            }
        }
        let mut stars = Vec::new();
        depth_first(cat, root, &mut stars);
        // With no star age in the catalog SE draws the system's age first (FUN_1404f62e0).
        if !stars.iter().any(|&j| cat.stars[j as usize].block.has("Age")) {
            s.uniform_a();
        }
        for &star in &stars {
            let kids = linked_children(cat, star);
            if kids.is_empty() {
                continue;
            }
            let desc: Vec<Child> = kids
                .iter()
                .map(|&(r, t)| {
                    let b = block_of(cat, r);
                    Child { se_type: t, orbit: given_angles(b), msini: b.f64("Msini").map(|m| m as f32) }
                })
                .collect();
            for (&(r, _), angles) in kids.iter().zip(generate_angles(&mut s, &desc)) {
                if let (ObjectRef::Body(i), (Some(_), _) | (_, Some(_))) = (r, angles) {
                    generated.push((i as usize, angles));
                }
            }
        }
    }
    for (i, b) in cat.bodies.iter().enumerate() {
        if !matches!(b.tag, BodyTag::Moon | BodyTag::DwarfMoon) || cat.find(&b.parent).is_none() {
            continue;
        }
        if let Some(given @ ((None, _) | (_, None))) = given_angles(&b.block) {
            let seed = crate::surface_gen::catalog_seeds(&b.names.join("/"), &b.parent).0;
            generated.push((i, moon_angles(seed, given)));
        }
    }
    for (i, (incl, node)) in generated {
        if let Some(o) = cat.bodies[i].orbit.as_mut() {
            o.inclination = incl.unwrap_or(o.inclination);
            o.ascending_node = node.unwrap_or(o.ascending_node);
        }
    }
    for b in &mut cat.bodies {
        let blk = &b.block;
        let Some(msini) = blk.f64("Msini") else { continue };
        if ["Mass", "MassEarth", "MassKg", "MassJup", "MassSol"].iter().any(|k| blk.has(k)) {
            continue;
        }
        if let Some(o) = &b.orbit {
            b.mass_kg = msini_mass(msini as f32, o.inclination) * EARTH_MASS_KG;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_seed_matches_se_log() {
        // Seeds from SE's log ("Creating planetary system of star ..."), positions from its catalogs.
        let proxima = crate::sc::parse("RA 14.49526367\nDec -62.679485\nDist 1.29");
        assert_eq!(system_seed(&proxima), 1_983_888_000);
        let sirius = crate::sc::parse("RA 6.7524722\nDec -16.716111\nDist 2.587687");
        assert_eq!(system_seed(&sirius) as i32, -308_782_848);
        assert_eq!(system_seed(&crate::sc::parse("Class \"G2V\"")), 0);
    }

    #[test]
    fn ndtri_inverts_the_normal_cdf() {
        for p in [1e-12, 0.01, 0.1353, 0.3, 0.5, 0.8, 0.95, 1.0 - 1e-9] {
            let x = ndtri(p);
            let cdf = 0.5 * libm::erfc(-x / SQRT_2);
            assert!((cdf / p - 1.0).abs() < 1e-9, "{p} -> {x} -> {cdf}");
        }
    }

    #[test]
    fn proxima_planets_get_se_inclinations_and_masses() {
        let seed = 1_983_888_000;
        let mut s = Streams { a: seed, b: seed };
        // The barycentre "Proxima Cen" links the star; the star links d, b and c (catalog order).
        let star = Child { se_type: 3, orbit: None, msini: None };
        assert!(generate_angles(&mut s, &[star]).iter().all(|a| a == &(None, None)));
        let planet = |msini: f32, given: Option<(f64, f64)>| Child {
            se_type: 4,
            orbit: Some(given.map_or((None, None), |(i, n)| (Some(i * DEG), Some(n * DEG)))),
            msini: Some(msini),
        };
        let kids = [planet(0.26030351, None), planet(1.071, None), planet(5.68, Some((133.0, 135.0)))];
        let out = generate_angles(&mut s, &kids);
        let (d, b) = (out[0].0.unwrap(), out[1].0.unwrap());
        assert!((d / DEG - 131.981_303_8).abs() < 1e-6, "{}", d / DEG);
        assert!((b / DEG - 132.102_037_9).abs() < 1e-6, "{}", b / DEG);
        assert!(out[2] == (None, None));
        // SE's info panel: 0.35017 and 1.4435 Earth masses.
        assert!((msini_mass(0.26030351, d) - 0.35017).abs() < 5e-6);
        assert!((msini_mass(1.071, b) - 1.4435).abs() < 5e-5);
    }

    #[test]
    fn moon_angles_fill_only_what_is_missing() {
        for seed in [0u32, 1, 0xdead_beef, u32::MAX] {
            let (i, n) = moon_angles(seed, (None, None));
            assert!(i.unwrap().abs() <= 1.5 * DEG && (0.0..2.0 * PI).contains(&n.unwrap()));
            // The node draw comes second, so a given inclination leaves the first draw to it.
            assert_eq!(moon_angles(seed, (Some(0.1), None)), (None, Some(lcg(seed) as f64 * (2.0 * PI) * INV_2_32)));
            assert_eq!(moon_angles(seed, (Some(0.1), Some(0.2))), (None, None));
        }
    }

    #[test]
    #[ignore = "reads the catalogs from the install"]
    fn proxima_masses_from_catalog_match_se() {
        let vfs = crate::vfs::Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
        let mut cat = Catalog::load(&vfs);
        complete_catalog_orbits(&mut cat);
        // 51 Peg b has no sibling with a known inclination: its mean comes from stream A.
        for (name, mass) in [("Proxima b", 1.4435), ("Proxima d", 0.35017), ("Dimidium", 0.47171 * 317.8284)] {
            let Some(ObjectRef::Body(i)) = cat.find(name) else { panic!("{name} missing") };
            let m = cat.bodies[i as usize].mass_kg / EARTH_MASS_KG;
            assert!((m / mass - 1.0).abs() < 5e-5, "{name}: {m}");
        }
    }
}
