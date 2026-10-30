//! SE's life generator (FUN_1404faf40), run once a system's bodies are generated and completed,
//! with the development stages of FUN_1404c9d90.
//!
//! - **Origin**: two stream-A draws per body. Solid bodies (bulk below Neptune) by volatiles
//!   class: arid with ProbLifeOrgDesert (0.02) / ProbLifeExoDesert (0.002); lacustrine or marine
//!   with ProbLifeOrgTerra (0.3) / ProbLifeExotTerra (0.1); oceanic or superoceanic, when the ocean
//!   has a rocky floor (volatiles flag 0x10), with ProbLifeOrgOceania (0.3) / ProbLifeExoOceania
//!   (0.03). Organic life needs a surface temperature of 250-640 K, exotic life 80-111 K. An ice
//!   world (FUN_1404ca8d0: Aquaria, `T_eq` below 160 K, no hydrogen or helium, some ice, under 1
//!   GPa at the ice's base) larger than 1200 km gets subglacial life with ProbLifeOrgIceWorld
//!   (0.05), else ProbLifeExoIceWorld (0.005). Giants: aerial life with ProbLifeOrgGiant (0.01)
//!   at 150-400 K, ProbLifeExoGiant (0.002) at 60-120 K.
//! - **Panspermia**: from the system seed (stream A `seed + 0x8e5fc`, B `seed + 0xd114b`), organic
//!   and exotic spores leave with ProbPanspEject* times the number of worlds of each kind with
//!   life; every other body of the kind then takes them up with ProbPanspInsert* (Terra 0.1,
//!   Oceania 0.2, Desert 0.04, IceWorld 0.001, Giant 0.05) within the same temperature windows.
//!   Exotic life displaces organic.
//! - **Stages** (FUN_1404c9d90): unicellular after `N(0.6, 0.25)` Gyr (exotic `N(1.5, 0.4)`),
//!   multicellular after `N(3.5, 0.7)` (`N(4.5, 1)`), complex after `N(4, 0.5)` (`N(6.5, 1.5)`),
//!   each at least 0.1 Gyr; younger systems lose the life. Biomes: marine on lacustrine to
//!   superoceanic worlds; terrestrial on arid ones, and with complex life on lacustrine and marine
//!   ones too; subglacial on ice worlds; aerial on giants (SE's exotic giants are always at least
//!   unicellular).
//!
//! Assumed: the two origin draws per body come from a stream of the system seed (their place in
//! SE's stream is not replicated), bodies in catalog order; the surface lava level SE also tests
//! (below 0.1) is never generated, so it passes.

use crate::catalog::{BodyTag, Catalog, ObjectRef};
use crate::surface_gen::{bulk, Classes, SeRng};

/// main-def.cfg `LifeGenerator`, exe globals 0x14075f8f4 .. 0x14075f878 (FUN_140280ec0).
const ORG: Probs = Probs { terra: 0.3, oceania: 0.3, desert: 0.02, ice: 0.05, giant: 0.01 };
const EXO: Probs = Probs { terra: 0.1, oceania: 0.03, desert: 0.002, ice: 0.005, giant: 0.002 };
const EJECT: Probs = Probs { terra: 1.0, oceania: 0.01, desert: 1.0, ice: 0.005, giant: 0.0001 };
const INSERT: Probs = Probs { terra: 0.1, oceania: 0.2, desert: 0.04, ice: 0.001, giant: 0.05 };
/// LifeOrgAge* / LifeExoAge*: mean and dispersion (Gyr) of the unicellular, multicellular and
/// complex stages.
const ORG_AGES: [(f32, f32); 3] = [(0.6, 0.25), (3.5, 0.7), (4.0, 0.5)];
const EXO_AGES: [(f32, f32); 3] = [(1.5, 0.4), (4.5, 1.0), (6.5, 1.5)];

struct Probs {
    terra: f32,
    oceania: f32,
    desert: f32,
    ice: f32,
    giant: f32,
}

/// What the generator reads of a body.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LifeIn {
    /// No generated life (the catalog gives `Life` or `NoLife`, or SE's completion did not run);
    /// the body still takes its two draws.
    pub skip: bool,
    pub bulk: i32,
    /// Volatiles class ([`crate::surface_gen::volq_class`]).
    pub volq: i32,
    pub t_surf: f32,
    /// The ocean rests on rock (volatiles flag 0x10).
    pub rock_floor: bool,
    pub ice_world: bool,
    pub radius_km: f32,
    pub age_gyr: f32,
}

/// One kind of life a body ends up with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LifeOut {
    pub exotic: bool,
    pub multicellular: bool,
    pub biome: &'static str,
    pub panspermia: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Terra,
    Oceania,
    Desert,
    Ice,
    Giant,
}

fn prob(p: &Probs, k: Kind) -> f32 {
    match k {
        Kind::Terra => p.terra,
        Kind::Oceania => p.oceania,
        Kind::Desert => p.desert,
        Kind::Ice => p.ice,
        Kind::Giant => p.giant,
    }
}

fn org_window(t: f32) -> bool {
    250.0 < t && t < 640.0
}

fn exo_window(t: f32) -> bool {
    80.0 < t && t < 111.0
}

/// SE's ice world (FUN_1404ca8d0): Aquaria below 160 K without hydrogen or helium, with ice whose
/// weight at its base (`g M_ice / 4 pi R^2`) stays under 1 GPa.
pub(super) fn ice_world(bulk_class: i32, t_eq: f32, comp: &[f32; 6], mass_earth: f32, radius_km: f32) -> bool {
    use crate::climate::part;
    if bulk_class != bulk::AQUARIA || t_eq >= 160.0 || comp[part::GAS] + comp[part::GAS2] > 0.0 || comp[part::ICE] == 0.0 {
        return false;
    }
    let r_e = radius_km / 6378.14;
    let g = mass_earth / (r_e * r_e) * 9.806_65;
    let r_m = (r_e * 6378.14 * 1000.0) as f64;
    let p = (g * mass_earth * 5.972_365e24 * comp[part::ICE]) as f64 / (r_m * 12.566_370_614_359_172 * r_m);
    p < 1e9
}

/// The origin kind of a solid body's volatiles class, if any.
fn solid_kind(b: &LifeIn) -> Option<Kind> {
    match b.volq {
        1 => Some(Kind::Desert),
        2 | 3 => Some(Kind::Terra),
        4 | 5 if b.rock_floor => Some(Kind::Oceania),
        _ => None,
    }
}

/// SE's life generator over a system's bodies (in catalog order) with system seed `seed`.
pub(super) fn generate(seed: u32, bodies: &[LifeIn]) -> Vec<Vec<LifeOut>> {
    let mut r1 = SeRng::new(seed ^ 0x4c49_4645, seed ^ 0x4c49_4645);
    // Pass 1: origin, and the number of worlds of each kind with each life.
    let mut origin = vec![(false, false); bodies.len()];
    let (mut n_org, mut n_exo) = ([0u32; 5], [0u32; 5]);
    let idx = |k: Kind| k as usize;
    for (b, o) in bodies.iter().zip(origin.iter_mut()) {
        let (u1, u2) = (r1.uniform(1.0), r1.uniform(1.0));
        if b.skip {
            continue;
        }
        if b.bulk < bulk::NEPTUNE {
            if let Some(k) = solid_kind(b) {
                if u1 <= prob(&ORG, k) && org_window(b.t_surf) {
                    o.0 = true;
                    n_org[idx(k)] += 1;
                }
                if u2 <= prob(&EXO, k) && exo_window(b.t_surf) {
                    o.1 = true;
                    n_exo[idx(k)] += 1;
                }
            }
            if b.ice_world && b.radius_km > 1200.0 {
                if u1 <= ORG.ice {
                    o.0 = true;
                    n_org[idx(Kind::Ice)] += 1;
                } else if u2 <= EXO.ice {
                    o.1 = true;
                    n_exo[idx(Kind::Ice)] += 1;
                }
            }
        } else {
            if u1 <= ORG.giant && 150.0 < b.t_surf && b.t_surf < 400.0 {
                o.0 = true;
                n_org[idx(Kind::Giant)] += 1;
            }
            if u2 <= EXO.giant && 60.0 < b.t_surf && b.t_surf < 120.0 {
                o.1 = true;
                n_exo[idx(Kind::Giant)] += 1;
            }
        }
    }
    // Panspermia: do spores of each life leave their worlds?
    let mut r = SeRng::new(seed.wrapping_add(0x8e5fc), seed.wrapping_add(0xd114b));
    let kinds = [Kind::Terra, Kind::Oceania, Kind::Desert, Kind::Ice, Kind::Giant];
    let mut spread = |n: &[u32; 5]| kinds.iter().fold(false, |any, &k| r.uniform(1.0) <= n[idx(k)] as f32 * prob(&EJECT, k) || any);
    let (org_spores, exo_spores) = (spread(&n_org), spread(&n_exo));
    // Pass 2: take-up and the stages.
    bodies
        .iter()
        .zip(&origin)
        .map(|(b, &(org0, exo0))| {
            let (u1, u2) = (r.uniform(1.0), r.uniform(1.0));
            if b.skip {
                return Vec::new();
            }
            let (mut org, mut exo) = (false, false);
            let take = |spores: bool, u: f32, k: Kind| spores && u <= prob(&INSERT, k);
            if b.bulk < bulk::NEPTUNE {
                if let Some(k) = solid_kind(b) {
                    org = org0 || (take(org_spores, u1, k) && org_window(b.t_surf));
                    if exo0 || (take(exo_spores, u2, k) && exo_window(b.t_surf)) {
                        (org, exo) = (false, true);
                    }
                }
                if b.ice_world && b.radius_km > 1200.0 {
                    org = org0 || take(org_spores, u1, Kind::Ice);
                    exo = exo0 || take(exo_spores, u2, Kind::Ice);
                }
            } else {
                org = org0 || (take(org_spores, u1, Kind::Giant) && 150.0 < b.t_surf && b.t_surf < 400.0);
                if exo0 || (take(exo_spores, u2, Kind::Giant) && 60.0 < b.t_surf && b.t_surf < 120.0) {
                    (org, exo) = (false, true);
                }
            }
            let mut out = Vec::new();
            if org {
                out.extend(stage(&mut r, b, false, !org0));
            }
            if exo {
                out.extend(stage(&mut r, b, true, !exo0));
            }
            out
        })
        .collect()
}

/// FUN_1404c9d90 for one kind of life: its stage and biome, or none when the system is too young.
fn stage(r: &mut SeRng, b: &LifeIn, exotic: bool, panspermia: bool) -> Option<LifeOut> {
    let ages = if exotic { EXO_AGES } else { ORG_AGES };
    let [uni, multi, complex] = ages.map(|(m, s)| r.gauss(m, s).clamp(0.1, 100.0));
    let age = b.age_gyr;
    let life = |multicellular: bool, biome: &'static str| Some(LifeOut { exotic, multicellular, biome, panspermia });
    let giant = b.bulk == bulk::NEPTUNE || b.bulk == bulk::JUPITER;
    if giant {
        if complex < age || multi < age {
            return life(true, "Aerial");
        }
        // SE tests its unicellular threshold against zero for exotic giants.
        return if (exotic && uni != 0.0) || (!exotic && uni < age) { life(false, "Aerial") } else { None };
    }
    if b.ice_world {
        return if complex < age || multi < age {
            life(true, "Subglacial")
        } else if uni < age {
            life(false, "Subglacial")
        } else {
            None
        };
    }
    if !(1..=4).contains(&b.bulk) {
        // SE leaves the origin flag without a stage.
        return life(false, "");
    }
    let marine = (2..=5).contains(&b.volq);
    let ground = if complex < age { (1..=3).contains(&b.volq) } else { b.volq == 1 };
    let multicellular = if complex < age || multi < age {
        true
    } else if uni < age {
        false
    } else {
        return None;
    };
    life(multicellular, match (marine, ground) {
        (true, true) => "Marine/Terrestrial",
        (true, false) => "Marine",
        (false, true) => "Terrestrial",
        _ => "",
    })
}

/// SE's life for the generated bodies of the system under catalog star `root` (procedural
/// systems and the generated systems of catalog stars): each body is completed as the engine
/// will complete it ([`crate::climate::complete`]) and given `Life` blocks. Returns the number of
/// bodies with life.
pub fn apply(cat: &mut Catalog, root: u32, seed: u32, jd: f64) -> usize {
    let Some(tables) = cat.climate.clone() else { return 0 };
    let name = cat.stars[root as usize].names[0].clone();
    let ids: Vec<usize> = cat
        .system_members(&name)
        .into_iter()
        .filter_map(|o| match o {
            ObjectRef::Body(i) if matches!(cat.bodies[i as usize].tag, BodyTag::Planet | BodyTag::DwarfPlanet | BodyTag::Moon | BodyTag::DwarfMoon) => Some(i as usize),
            _ => None,
        })
        .collect();
    let eph = crate::ephemeris::Ephemeris::new(cat);
    let stars = crate::climate::system_stars(cat, root);
    let inputs: Vec<LifeIn> = ids
        .iter()
        .map(|&i| {
            let b = &cat.bodies[i];
            let given = b.block.sub("Life").is_some() || b.block.bool("NoLife").unwrap_or(false);
            let light = crate::climate::star_light_from(&eph, cat, ObjectRef::Body(i as u32), jd, &stars);
            let completed = crate::climate::catalog_body_in(cat, i, &light).and_then(|bi| crate::climate::complete(&tables, &bi).map(|c| (bi, c)));
            match completed {
                Some((bi, c)) if !given => {
                    let classes = Classes::new(bi.obj_type, c.bulk, c.t_class, c.pressure_atm, bi.mass_earth, c.ocean_info());
                    LifeIn {
                        skip: false,
                        bulk: c.bulk,
                        volq: classes.volq,
                        t_surf: c.t_surf,
                        rock_floor: c.volatiles.flags & 0x10 != 0,
                        ice_world: ice_world(c.bulk, c.t_eq, &c.composition, bi.mass_earth, bi.radius_km),
                        radius_km: bi.radius_km,
                        age_gyr: bi.age_gyr,
                    }
                }
                _ => LifeIn { skip: true, ..Default::default() },
            }
        })
        .collect();
    drop(eph);
    let mut n = 0;
    for (i, lives) in ids.into_iter().zip(generate(seed, &inputs)) {
        n += !lives.is_empty() as usize;
        cat.bodies[i].block.items.extend(lives.iter().map(life_item));
    }
    n
}

/// A body's `Life { Class Type Biome [Panspermia] }` item.
fn life_item(l: &LifeOut) -> crate::sc::Item {
    let mut text = format!("Class \"{}\"\n", if l.exotic { "Exotic" } else { "Organic" });
    if !l.biome.is_empty() || l.multicellular {
        text += &format!("Type \"{}\"\n", if l.multicellular { "Multicellular" } else { "Unicellular" });
    }
    if !l.biome.is_empty() {
        text += &format!("Biome \"{}\"\n", l.biome);
    }
    if l.panspermia {
        text += "Panspermia true\n";
    }
    crate::sc::Item { key: "Life".into(), args: Vec::new(), block: Some(crate::sc::parse(&text)), line: 0 }
}

/// [`apply`] for a generated system before it joins the engine's catalog (off the main thread, and
/// for the star browser): `stars` are the system's star records, root first (its mass set to
/// `root_mass_sol` for a multiple system), `doc` the generator's parsed script, which gets the
/// `Life` items. Returns the bodies with life.
pub fn apply_to_doc(stars: &[crate::catalog::StarRecord], tables: std::sync::Arc<crate::climate::ClimateTables>, root_mass_sol: Option<f64>, doc: &mut crate::sc::Block, jd: f64) -> usize {
    let Some(root) = stars.first() else { return 0 };
    let mut cat = Catalog::new();
    cat.climate = Some(tables);
    for s in stars {
        cat.add_star_record(s.clone());
    }
    if let Some(m) = root_mass_sol {
        cat.stars[0].mass_kg = m * crate::units::SOLAR_MASS_KG;
    }
    let b0 = cat.bodies.len();
    cat.load_doc(doc.clone(), crate::climate::PROCEDURAL_SOURCE);
    cat.finalize();
    let n = apply(&mut cat, 0, super::se_system_seed(root), jd);
    let lives: std::collections::HashMap<&str, Vec<&crate::sc::Item>> = cat.bodies[b0..]
        .iter()
        .filter_map(|b| {
            let items: Vec<_> = b.block.items.iter().filter(|i| i.key == "Life").collect();
            (!items.is_empty()).then(|| (b.names[0].as_str(), items))
        })
        .collect();
    for it in doc.items.iter_mut() {
        if let (Some(items), Some(blk)) = (it.name().and_then(|n| lives.get(n)), it.block.as_mut()) {
            blk.items.extend(items.iter().map(|&i| i.clone()));
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earth(age: f32) -> LifeIn {
        LifeIn { skip: false, bulk: bulk::TERRA, volq: 3, t_surf: 288.0, rock_floor: true, ice_world: false, radius_km: 6371.0, age_gyr: age }
    }

    #[test]
    fn rates_and_stages() {
        // Marine Earths: about ProbLifeOrgTerra of them get life (plus panspermia); old ones
        // multicellular, young ones none.
        let (mut with, mut multi, n) = (0, 0, 3000u32);
        for s in 0..n {
            let out = generate(s.wrapping_mul(0x9e37_79b9), &[earth(4.6)]);
            if let Some(l) = out[0].first() {
                with += 1;
                multi += l.multicellular as usize;
                assert!(!l.exotic && l.biome.starts_with("Marine"));
            }
        }
        let f = with as f64 / n as f64;
        assert!((0.27..0.36).contains(&f), "life share {f}");
        assert!(multi as f64 / with as f64 > 0.8);
        assert!((0..500u32).all(|s| generate(s, &[earth(0.05)])[0].is_empty()));
        // Too hot, or skipped: none.
        let hot = LifeIn { t_surf: 700.0, ..earth(4.6) };
        assert!((0..500u32).all(|s| generate(s, &[hot])[0].is_empty()));
        // Many living worlds spread spores to a lifeless desert neighbour.
        let mut sys = vec![earth(4.6); 12];
        sys.push(LifeIn { volq: 1, ..earth(4.6) });
        let spread = (0..400u32).filter(|&s| generate(s, &sys)[12].iter().any(|l| l.panspermia)).count();
        assert!(spread > 0, "no panspermia");
    }

    /// Life on completed generated systems (reads SE's gas table from the install:
    /// `cargo test -p se_core se_life -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn generated_systems_get_life() {
        let tables = std::sync::Arc::new(crate::climate::ClimateTables::from_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("tables"));
        let (mut living, mut kinds) = (0, std::collections::BTreeMap::<String, usize>::new());
        for k in 0..60u64 {
            let name = format!("RS 0-9-9-{k}");
            let s = crate::procgen::StarParams {
                name: name.clone(),
                mass_sol: 1.0,
                lum_bol: 1.0,
                teff: 5778.0,
                radius_sol: 1.0,
                age_gyr: 4.6,
                feh: 0.0,
                kind: crate::star::StarKind::Normal,
                class: None,
                c_to_o: None,
                se_seed: Some((k as u32).wrapping_mul(0x9e37_79b9)),
            };
            let g = crate::procgen::generate_planets(&s, k);
            let mut cat = Catalog::new();
            cat.climate = Some(tables.clone());
            cat.load_sc(&format!("Star \"{name}\" {{ RA 1 Dec 2 Dist 10 Class \"G2V\" MassSol 1 Age 4.6 }}"), "s.sc");
            let star = cat.stars[0].clone();
            cat.load_doc(crate::sc::parse(&g.sc), crate::climate::PROCEDURAL_SOURCE);
            cat.finalize();
            let here = crate::procgen::apply_life(&mut cat, 0, crate::time::J2000);
            // Off the main thread on the parsed script (the engine's path): the same life.
            let mut doc = crate::sc::parse(&g.sc);
            assert_eq!(apply_to_doc(&[star], tables.clone(), None, &mut doc, crate::time::J2000), here, "{name}");
            living += here;
            for b in &cat.bodies {
                for l in b.block.all("Life").filter_map(|i| i.block()) {
                    let key = format!("{} {} {}", l.str("Class").unwrap_or(""), l.str("Type").unwrap_or(""), l.str("Biome").unwrap_or(""));
                    *kinds.entry(key).or_default() += 1;
                }
            }
        }
        eprintln!("bodies with life in 60 Sun-like systems: {living}: {kinds:?}");
        assert!(living > 0);
    }

    #[test]
    fn ice_worlds() {
        // Europa-like: 0.008 Me, 1560 km, 8% ice, 100 K.
        let comp = [0.0, 0.0, 0.08, 0.6, 0.0, 0.32];
        assert!(ice_world(bulk::AQUARIA, 100.0, &comp, 0.008, 1560.0));
        assert!(!ice_world(bulk::TERRA, 100.0, &comp, 0.008, 1560.0));
        assert!(!ice_world(bulk::AQUARIA, 200.0, &comp, 0.008, 1560.0));
        let w = LifeIn { bulk: bulk::AQUARIA, volq: 6, t_surf: 100.0, rock_floor: false, ice_world: true, radius_km: 1560.0, ..earth(4.6) };
        let got = (0..4000u32).filter_map(|s| generate(s, &[w]).remove(0).into_iter().next()).collect::<Vec<_>>();
        assert!(!got.is_empty() && got.iter().all(|l| l.biome == "Subglacial"));
    }
}
