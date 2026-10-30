//! Procedural planetary-system generator.
//!
//! Given one star ([`StarParams`]) and a seed, [`generate_system_sc`] returns SpaceEngine
//! catalog script text (`.sc`, the same dialect SE's "export planet script" writes) that
//! the engine loads through [`crate::catalog::Catalog::load_sc`]. Planets, dwarf planets,
//! moons and belt asteroids come out with orbits, rotation, masses and radii; SE's completion
//! chain then gives planets and moons their composition, atmosphere, ocean and clouds.
//!
//! The architecture is SE's ([`se_layout`]): planet slots, spacing, types, masses, radii,
//! dwarf planets with their asteroid belts, the number of moons and a single star's comets
//! ([`se_comets`]). Moon orbits and the asteroids' sizes are still designed here, on the port's
//! own SplitMix64 streams. The generator is a pure function of `(star, seed)`; see
//! `docs/procgen.md`.

use std::fmt::Write as _;

use glam::DVec3;

mod se_comets;
mod se_layout;
mod se_multiple;

pub use se_comets::{comet_name, tail_defaults, tail_particles, CometTail, CometTailGiven, TailParticle};

use crate::star::StarKind;
use crate::surface_gen::{bulk, BULK_NAMES};
use crate::units::{EARTH_MASS_KG, EARTH_RADIUS_KM, G, METERS_PER_AU, SECONDS_PER_YEAR, SOLAR_MASS_KG};

// ---------------------------------------------------------------------------------------
// RNG
// ---------------------------------------------------------------------------------------

fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Small deterministic RNG (SplitMix64) with forkable sub-streams.
#[derive(Debug, Clone)]
pub struct Rng {
    base: u64,
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let base = mix64(seed ^ 0x5851_F42D_4C95_7F2D);
        Self { base, state: base }
    }

    /// Seed from a numeric seed and a name (so one seed gives different systems per star).
    pub fn from_name(seed: u64, name: &str) -> Self {
        Self::new(mix64(seed) ^ fnv1a(name))
    }

    /// Independent stream derived from this RNG's *seed* (not its position), so inserting
    /// draws elsewhere never shifts what a fork produces.
    pub fn fork(&self, tag: u64) -> Rng {
        Rng::new(self.base ^ mix64(tag.wrapping_mul(0xD6E8_FEB8_6659_FD93).wrapping_add(0x1234_5678_9ABC_DEF1)))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix64(self.state)
    }

    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }

    /// Log-uniform in [lo, hi] (both > 0).
    pub fn log_range(&mut self, lo: f64, hi: f64) -> f64 {
        (lo.ln() + (hi.ln() - lo.ln()) * self.f64()).exp()
    }

    /// Standard normal (Box-Muller, one value per call).
    pub fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.f64();
        let u2 = self.f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    /// Uniform integer in [0, n).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }

    fn rayleigh(&mut self, sigma: f64) -> f64 {
        sigma * (-2.0 * (1.0 - self.f64()).ln()).sqrt()
    }

    /// Index drawn with the given (unnormalised) weights.
    fn weighted(&mut self, w: &[f64]) -> usize {
        let total: f64 = w.iter().sum();
        let mut x = self.f64() * total;
        for (i, wi) in w.iter().enumerate() {
            if x < *wi {
                return i;
            }
            x -= wi;
        }
        w.len().saturating_sub(1)
    }
}

// ---------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------

/// The central star, as the engine knows it. All values are in solar units except `teff` (K).
#[derive(Debug, Clone)]
pub struct StarParams {
    /// Name used as `ParentBody` of the planets and as the planet-name prefix.
    pub name: String,
    pub mass_sol: f64,
    /// Bolometric luminosity in solar luminosities.
    pub lum_bol: f64,
    pub teff: f64,
    pub radius_sol: f64,
    pub age_gyr: f64,
    /// Metallicity [Fe/H] (dex).
    pub feh: f64,
    pub kind: StarKind,
    /// The star's spectral class, when known (SE's layout reads its luminosity class).
    pub class: Option<crate::star::SpectralClass>,
}

/// One generated body, as a compact summary (the full definition is in the `.sc` text).
#[derive(Debug, Clone)]
pub struct BodySummary {
    pub name: String,
    /// `.sc` tag: Planet, DwarfPlanet, Moon, DwarfMoon or Asteroid.
    pub tag: &'static str,
    pub parent: String,
    /// Bulk class written to `Class` (Terra, Ferria, Carbonia, Aquaria, Neptune, Jupiter,
    /// Chthonia, Asteroid).
    pub class: String,
    /// Full descriptive class, e.g. "Temperate mesobaric marine terra with life".
    pub class_desc: String,
    pub mass_me: f64,
    pub radius_km: f64,
    /// Semi-major axis around the parent (AU for planets/asteroids, also AU for moons).
    pub a_au: f64,
    pub ecc: f64,
    /// Heliocentric distance used for insolation (equals `a_au` except for moons).
    pub helio_a_au: f64,
    pub t_eq_k: f64,
    pub t_surf_k: f64,
    pub pressure_atm: f64,
    pub albedo_bond: f64,
    pub tidally_locked: bool,
    pub has_atmosphere: bool,
    pub has_ocean: bool,
    pub has_life: bool,
    pub life: Option<Life>,
    pub has_rings: bool,
}

/// Generator output: the `.sc` text plus a summary of every body (for tools and tests).
#[derive(Debug, Clone)]
pub struct GeneratedSystem {
    pub sc: String,
    pub bodies: Vec<BodySummary>,
    /// The component stars of a multiple system, (name, class) (empty for a single star).
    pub stars: Vec<(String, String)>,
    /// A multiple system's total mass (solar masses), which its root barycentre must carry.
    pub root_mass_sol: Option<f64>,
}

/// Generate the `.sc` text for the planetary system of `star`. Deterministic in `(star, seed)`.
pub fn generate_system_sc(star: &StarParams, seed: u64) -> String {
    generate_system(star, seed).sc
}

/// Like [`generate_system_sc`] but also returns per-body summaries. A procedural star may become
/// SE's multiple system ([`is_multiple`]).
pub fn generate_system(star: &StarParams, seed: u64) -> GeneratedSystem {
    generate(star, seed, true)
}

/// The planets SE generates for a catalog star without catalog bodies: the star stays as the
/// catalog has it (SE splits only its procedural stars).
pub fn generate_planets(star: &StarParams, seed: u64) -> GeneratedSystem {
    generate(star, seed, false)
}

fn generate(star: &StarParams, seed: u64, multiples: bool) -> GeneratedSystem {
    let ctx = Ctx::new(star);
    let mut rng = Rng::from_name(seed, &star.name);
    let seed32 = system_seed(star, seed);
    let mut sc = String::new();
    let _ = writeln!(sc, "// Procedural planetary system of \"{}\" (se_core::procgen, seed {})", ctx.name, seed);
    let _ = writeln!(
        sc,
        "// Star: {:.3} Msun, {:.4} Lsun, Teff {:.0} K, R {:.3} Rsun, age {:.2} Gyr, [Fe/H] {:+.2}, regime {:?}",
        ctx.mass, ctx.lum, ctx.teff, ctx.radius_sol, ctx.age, ctx.feh, ctx.regime
    );
    let mut out = GeneratedSystem { sc: String::new(), bodies: Vec::new(), stars: Vec::new(), root_mass_sol: None };
    // SE's black holes of 400 solar masses or more take another generator (FUN_1404fb950).
    let multiple = if big_black_hole(star) || !multiples { None } else { se_multiple::build(&se_root(star, &ctx), seed32) };
    match multiple {
        None => {
            let (planets, asteroids, comets) = se_bodies(&mut rng, star, &ctx, seed32);
            host_bodies(&mut rng, &ctx, planets, asteroids, &mut sc, &mut out.bodies);
            host_comets(&ctx, &comets, &mut sc, &mut out.bodies);
        }
        Some(comps) => {
            out.root_mass_sol = Some(comps[0].star.mass_sol);
            let name = |i: usize| comp_name(&star.name, &comps[i].suffix);
            for (i, c) in comps.iter().enumerate().skip(1) {
                emit_component(&mut sc, &name(i), &name(c.parent.unwrap_or(0)), c);
                if c.children.is_none() {
                    out.stars.push((name(i), c.class.clone()));
                }
            }
            let mut order: Vec<usize> = (0..comps.len()).filter(|&i| comps[i].slots > 0).collect();
            order.sort_by_key(|&i| comps[i].idx);
            let (mut n0, mut budget) = (0u32, se_layout::MAX_ASTEROIDS);
            for i in order {
                let c = &comps[i];
                let hp = StarParams {
                    name: name(i),
                    mass_sol: c.star.mass_sol,
                    lum_bol: c.star.lum_sol,
                    teff: c.star.teff,
                    radius_sol: c.star.radius_km / crate::units::SOLAR_RADIUS_KM,
                    age_gyr: star.age_gyr,
                    feh: star.feh,
                    kind: c.kind,
                    class: Some(crate::star::SpectralClass::parse(&c.class)),
                };
                let hctx = Ctx::new(&hp);
                let inner = |hot: bool| se_multiple::inner_au(&comps, i, hot);
                let sys = se_layout::place(&c.star, c.idx, c.slots, c.soi_au, &inner, seed32, n0, &mut budget);
                n0 += sys.planets.len() as u32;
                let mut hrng = rng.fork(0x51_0000 + c.idx as u64);
                let (planets, asteroids, _) = bodies_from(&mut hrng, &hctx, &sys);
                host_bodies(&mut hrng, &hctx, planets, asteroids, &mut sc, &mut out.bodies);
            }
        }
    }
    out.sc = sc;
    out
}

/// Whether SE's generator makes `star` a multiple system (its record is then a barycentre).
pub fn is_multiple(star: &StarParams, seed: u64) -> bool {
    let ctx = Ctx::new(star);
    !big_black_hole(star) && se_multiple::build(&se_root(star, &ctx), system_seed(star, seed)).is_some()
}

fn big_black_hole(star: &StarParams) -> bool {
    star.kind == StarKind::BlackHole && star.mass_sol >= 400.0
}

/// The 32-bit seed standing in for SE's system seed. The seed is mixed before the name's hash
/// joins it: the engine seeds a star by that same hash, which would otherwise cancel.
fn system_seed(star: &StarParams, seed: u64) -> u32 {
    (mix64(mix64(seed) ^ fnv1a(&star.name)) >> 32) as u32
}

/// A component's name: the system's plus SE's suffix ("A", "AB", ...).
fn comp_name(system: &str, suffix: &str) -> String {
    if suffix.is_empty() { system.to_string() } else { format!("{system} {suffix}") }
}

/// Planets (with moons) and asteroids of one host, through the port's physics, into `sc`.
fn host_bodies(rng: &mut Rng, ctx: &Ctx, mut planets: Vec<Body>, mut asteroids: Vec<Body>, sc: &mut String, summaries: &mut Vec<BodySummary>) {
    for (i, p) in planets.iter_mut().enumerate() {
        p.name = planet_name(&ctx.name, i + 1);
        p.parent = ctx.name.clone();
    }
    for (i, a) in asteroids.iter_mut().enumerate() {
        a.name = format!("{} S{}", ctx.name, i + 1);
        a.parent = ctx.name.clone();
    }
    let star_mass_kg = ctx.mass * SOLAR_MASS_KG;
    let mut moons_of: Vec<Vec<Body>> = Vec::with_capacity(planets.len());
    for (i, p) in planets.iter_mut().enumerate() {
        let mut prng = rng.fork(1000 + i as u64);
        // SE's size and class stay; the port's physics fills in what the emitter writes.
        let se = (p.radius_km, p.class, p.kind, p.tag);
        physics_planet(&mut prng, ctx, p, star_mass_kg);
        (p.radius_km, p.class, p.kind, p.tag) = se;
        let mut mrng = rng.fork(5000 + i as u64);
        let mut moons = gen_moons(&mut mrng, ctx, p);
        for (j, m) in moons.iter_mut().enumerate() {
            let mut r = rng.fork(9000 + (i as u64) * 64 + j as u64);
            physics_moon(&mut r, ctx, m, p);
        }
        moons_of.push(moons);
    }
    for (i, a) in asteroids.iter_mut().enumerate() {
        let mut r = rng.fork(20000 + i as u64);
        physics_asteroid(&mut r, ctx, a, star_mass_kg);
    }
    let _ = writeln!(sc, "// {}: {} planet(s)/dwarf planet(s), {} asteroid(s)", ctx.name, planets.len(), asteroids.len());
    for (p, moons) in planets.iter().zip(&moons_of) {
        emit_body(sc, p);
        summaries.push(p.summary());
        for m in moons {
            emit_body(sc, m);
            summaries.push(m.summary());
        }
    }
    for a in &asteroids {
        emit_body(sc, a);
        summaries.push(a.summary());
    }
}

/// A single star's comets ([`se_comets`]) into `sc`, as SE's generator leaves them: mass, radius,
/// albedo, rotation, magnitude, tail and orbit (with SE's period, which counts the comet's mass).
fn host_comets(ctx: &Ctx, comets: &[se_comets::SeComet], sc: &mut String, summaries: &mut Vec<BodySummary>) {
    if comets.is_empty() {
        return;
    }
    let _ = writeln!(sc, "\n// {}: {} comet(s)", ctx.name, comets.len());
    for (i, k) in comets.iter().enumerate() {
        let name = comet_name(&ctx.name, i + 1);
        let t = &k.tail;
        let _ = writeln!(sc, "\nComet \"{}\"\n{{", name);
        let _ = writeln!(sc, "\tParentBody     \"{}\"", ctx.name);
        let _ = writeln!(sc, "\tMass           {}\t// Earth masses", num(k.mass_me));
        let _ = writeln!(sc, "\tRadius         {}\t// km", num(k.radius_km));
        let _ = writeln!(sc, "\tAlbedoBond     {}", num(k.albedo));
        let _ = writeln!(sc, "\tAlbedoGeom     {}", num(k.albedo * 1.2));
        let _ = writeln!(sc, "\tRotationPeriod {}\t// hours", num(k.rot_h));
        let _ = writeln!(sc, "\tObliquity      {}", num(k.obliq_deg));
        let _ = writeln!(sc, "\tEqAscendNode   {}", num(k.eq_node_deg));
        let _ = writeln!(sc, "\tAbsMagn        {}", num(t.abs_magn as f64));
        let _ = writeln!(sc, "\tSlopeParam     {}", num(t.slope_param as f64));
        let _ = writeln!(sc, "\tCometTail\n\t{{");
        let _ = writeln!(sc, "\t\tMaxLength   {}\t// AU", num(t.max_length_km as f64 / 149_597_870.7));
        let _ = writeln!(sc, "\t\tGasToDust   {}", num(t.gas_to_dust as f64));
        let _ = writeln!(sc, "\t\tParticles   {}", t.particles);
        let _ = writeln!(sc, "\t\tGasBright   {}", num(t.gas_bright as f64));
        let _ = writeln!(sc, "\t\tDustBright  {}", num(t.dust_bright as f64));
        let c3 = |c: [f32; 3]| format!("({} {} {})", num(c[0] as f64), num(c[1] as f64), num(c[2] as f64));
        let _ = writeln!(sc, "\t\tGasColor    {}", c3(t.gas_color));
        let _ = writeln!(sc, "\t\tDustColor   {}", c3(t.dust_color));
        let _ = writeln!(sc, "\t}}");
        let _ = writeln!(sc, "\tOrbit\n\t{{");
        let _ = writeln!(sc, "\t\tRefPlane        \"Ecliptic\"");
        let _ = writeln!(sc, "\t\tSemiMajorAxis   {}\t// AU", num(k.a_au));
        let _ = writeln!(sc, "\t\tPeriod          {}\t// years", num(k.period_days / 365.25));
        let _ = writeln!(sc, "\t\tEccentricity    {}", num(k.ecc));
        let _ = writeln!(sc, "\t\tInclination     {}", num(k.inc_deg));
        let _ = writeln!(sc, "\t\tAscendingNode   {}", num(k.node_deg));
        let _ = writeln!(sc, "\t\tArgOfPericenter {}", num(k.argp_deg));
        let _ = writeln!(sc, "\t\tMeanAnomaly     {}", num(k.ma_deg));
        let _ = writeln!(sc, "\t}}\n}}");
        let t_eq = t_eq(ctx.lum, k.a_au, k.ecc, k.albedo);
        summaries.push(BodySummary {
            name,
            tag: "Comet",
            parent: ctx.name.clone(),
            class: "Comet".to_string(),
            class_desc: "Comet".to_string(),
            mass_me: k.mass_me,
            radius_km: k.radius_km,
            a_au: k.a_au,
            ecc: k.ecc,
            helio_a_au: k.a_au,
            t_eq_k: t_eq,
            t_surf_k: t_eq,
            pressure_atm: 0.0,
            albedo_bond: k.albedo,
            tidally_locked: false,
            has_atmosphere: false,
            has_ocean: false,
            has_life: false,
            life: None,
            has_rings: false,
        });
    }
}

/// A component of a multiple system as a catalog `Star` (or nested `Barycenter`) with its orbit
/// about its barycentre.
fn emit_component(out: &mut String, name: &str, parent: &str, c: &se_multiple::Comp) {
    let bary = c.children.is_some();
    let _ = writeln!(out, "\n{} \"{}\"\n{{", if bary { "Barycenter" } else { "Star" }, name);
    let _ = writeln!(out, "\tParentBody     \"{}\"", parent);
    if !bary {
        let _ = writeln!(out, "\tClass          \"{}\"", c.class);
        if c.star.teff > 0.0 {
            let _ = writeln!(out, "\tTeff           {}", num(c.star.teff));
            let _ = writeln!(out, "\tLumBol         {}", num(c.star.lum_sol));
        }
        let _ = writeln!(out, "\tRadius         {}\t// km", num(c.star.radius_km));
    }
    let _ = writeln!(out, "\tMassSol        {}", num(c.star.mass_sol));
    if let Some(o) = &c.orbit {
        let _ = writeln!(out, "\tOrbit\n\t{{");
        let _ = writeln!(out, "\t\tRefPlane        \"Ecliptic\"");
        let _ = writeln!(out, "\t\tSemiMajorAxis   {}", num(o.a_au));
        let _ = writeln!(out, "\t\tPeriod          {}\t// years", num(o.period_days / 365.25));
        let _ = writeln!(out, "\t\tEccentricity    {}", num(o.ecc));
        let _ = writeln!(out, "\t\tInclination     {}", num(o.inc_deg));
        let _ = writeln!(out, "\t\tAscendingNode   {}", num(o.node_deg));
        let _ = writeln!(out, "\t\tArgOfPericenter {}", num(o.argp_deg));
        let _ = writeln!(out, "\t\tMeanAnomaly     {}", num(o.ma_deg));
        let _ = writeln!(out, "\t}}");
    }
    let _ = writeln!(out, "}}");
}

/// SE-style procedural star name: `"RS <galaxy-code>-<level>-<node>-<index>"`.
///
/// Verified from SE's own data (`save/places-def.cfg`, `data/locale/*-db.cfg`): procedural
/// stars are named `RS` + the parent galaxy's code + `-<octree level>-<octree node id>-<star
/// index in node>`, e.g. `RS 8514-491-6-169551-68` (galaxy code `8514-491`, level 6, node
/// 169551, index 68) or `RS 0-8-11084762-901-49-7-384587-1004` (galaxy `0-8-11084762-901-49`).
/// `galaxy_code` is that galaxy part (an optional leading `RG ` is stripped). SE's own node
/// numbering was not reverse engineered, so `cell` is packed into a node id by 3-D Morton
/// interleaving (21 bits per axis, biased by 2^20); this is unique per cell, stable, and only
/// differs from SE's numbering in the number itself.
pub fn procedural_star_name(galaxy_code: &str, cell: [i64; 3], level: u8, index: u32) -> String {
    let code = galaxy_code.trim();
    let code = code.strip_prefix("RG ").unwrap_or(code);
    format!("RS {}-{}-{}-{}", code, level, cell_node_id(cell), index)
}

/// Morton-interleaved node id of an octree cell (see [`procedural_star_name`]).
pub fn cell_node_id(cell: [i64; 3]) -> u64 {
    fn spread(v: u64) -> u64 {
        let mut x = v & 0x1F_FFFF;
        x = (x | (x << 32)) & 0x1F00_0000_FFFF;
        x = (x | (x << 16)) & 0x1F_0000_FF00_00FF;
        x = (x | (x << 8)) & 0x100F_00F0_0F00_F00F;
        x = (x | (x << 4)) & 0x10C3_0C30_C30C_30C3;
        x = (x | (x << 2)) & 0x1249_2492_4924_9249;
        x
    }
    let b = |c: i64| (c + (1 << 20)).clamp(0, (1 << 21) - 1) as u64;
    spread(b(cell[0])) | (spread(b(cell[1])) << 1) | (spread(b(cell[2])) << 2)
}

/// Name of the `n`-th (1-based, outward) planet of a star. SE writes `"<star> <n>"` for a
/// single star and `"<star><n>"` directly after a component letter (`... A4`, `... B2`).
pub fn planet_name(star: &str, n: usize) -> String {
    let last = star.rsplit(' ').next().unwrap_or("");
    let is_component = star.contains(' ')
        && !last.is_empty()
        && last.len() <= 3
        && last.chars().all(|c| c.is_ascii_uppercase());
    if is_component { format!("{star}{n}") } else { format!("{star} {n}") }
}

// ---------------------------------------------------------------------------------------
// Classification words (SE: temperature / atmosphere / volatiles / bulk)
// ---------------------------------------------------------------------------------------

/// Temperature class word from the mean surface temperature in kelvin.
/// Words are SE's (torrid, hot, warm, temperate, cool, cold, frigid); thresholds are designed.
pub fn temperature_class(t_k: f64) -> &'static str {
    match t_k {
        t if t < 130.0 => "frigid",
        t if t < 240.0 => "cold",
        t if t < 273.0 => "cool",
        t if t < 310.0 => "temperate",
        t if t < 350.0 => "warm",
        t if t < 600.0 => "hot",
        _ => "torrid",
    }
}

/// Atmosphere class word from the surface pressure in atm (airless, infrabaric, hypobaric,
/// mesobaric, hyperbaric, ultrabaric). Words are SE's; thresholds are designed.
pub fn pressure_class(p_atm: f64) -> &'static str {
    match p_atm {
        p if p < 1e-7 => "airless",
        p if p < 1e-3 => "infrabaric",
        p if p < 0.1 => "hypobaric",
        p if p < 10.0 => "mesobaric",
        p if p < 500.0 => "hyperbaric",
        _ => "ultrabaric",
    }
}

/// Volatiles class word from the liquid-ocean and ice coverage fractions.
/// Words are SE's (arid, lacustrine, marine, oceanic, superoceanic, glacial, superglacial).
pub fn volatile_class(ocean_cover: f64, ice_cover: f64) -> &'static str {
    if ice_cover >= 0.95 {
        "superglacial"
    } else if ice_cover >= 0.3 {
        "glacial"
    } else if ocean_cover >= 0.97 {
        "superoceanic"
    } else if ocean_cover >= 0.75 {
        "oceanic"
    } else if ocean_cover >= 0.1 {
        "marine"
    } else if ocean_cover > 0.0 {
        "lacustrine"
    } else {
        "arid"
    }
}

/// Size prefix (micro, mini, sub, "", super, mega) for a bulk class and mass in Earth
/// masses. SE's size words exist in its locale files; the mass limits here are designed.
pub fn size_prefix(bulk: &str, mass_me: f64) -> &'static str {
    let pick = |limits: [f64; 5]| -> &'static str {
        const W: [&str; 6] = ["micro", "mini", "sub", "", "super", "mega"];
        W[limits.iter().take_while(|l| mass_me >= **l).count()]
    };
    match bulk {
        "Terra" | "Ferria" | "Carbonia" | "Aquaria" | "Chthonia" => pick([1e-3, 0.05, 0.5, 2.0, 10.0]),
        "Neptune" => match mass_me {
            m if m < 6.0 => "mini",
            m if m < 12.0 => "sub",
            m if m < 30.0 => "",
            m if m < 60.0 => "super",
            _ => "mega",
        },
        "Jupiter" => match mass_me {
            m if m < 30.0 => "mini",
            m if m < 150.0 => "sub",
            m if m < 1000.0 => "",
            m if m < 2500.0 => "super",
            _ => "mega",
        },
        _ => "",
    }
}

// ---------------------------------------------------------------------------------------
// Star context
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Regime {
    /// No planets (black holes, Wolf-Rayet, very massive stars).
    None,
    Normal,
    /// Evolved giant: only distant planets survive.
    Giant,
    WhiteDwarf,
    /// Pulsar planets.
    Neutron,
    BrownDwarf,
}

struct Ctx {
    name: String,
    regime: Regime,
    mass: f64,
    lum: f64,
    teff: f64,
    radius_sol: f64,
    age: f64,
    feh: f64,
    /// Snow line (AU), formation-based (uses at least the pre-main-sequence floor).
    a_snow: f64,
    /// Relative XUV flux (activity) multiplier used in atmospheric escape.
    xuv: f64,
}

impl Ctx {
    fn new(sp: &StarParams) -> Ctx {
        let fin = |x: f64, d: f64| if x.is_finite() { x } else { d };
        let mass = fin(sp.mass_sol, 1.0).clamp(0.01, 150.0);
        let lum = fin(sp.lum_bol, 1.0).max(1e-9);
        let radius_sol = fin(sp.radius_sol, 1.0).max(1e-5);
        let age = fin(sp.age_gyr, 4.6).clamp(0.001, 14.0);
        let feh = fin(sp.feh, 0.0).clamp(-2.5, 1.0);
        let teff = fin(sp.teff, 5778.0).max(500.0);
        let regime = match sp.kind {
            StarKind::BlackHole | StarKind::WolfRayet => Regime::None,
            StarKind::NeutronStar => Regime::Neutron,
            StarKind::WhiteDwarf => Regime::WhiteDwarf,
            StarKind::BrownDwarf => Regime::BrownDwarf,
            _ => {
                if mass < 0.075 {
                    Regime::BrownDwarf
                } else if mass > 12.0 {
                    Regime::None
                } else if radius_sol > 6.0 {
                    Regime::Giant
                } else {
                    Regime::Normal
                }
            }
        };
        let l_form = match regime {
            Regime::Normal | Regime::BrownDwarf => lum.max(0.04),
            Regime::Giant => mass.powf(3.5).clamp(0.04, 1e4),
            Regime::WhiteDwarf => 4.0,
            _ => 0.04,
        };
        let a_snow = 2.7 * l_form.sqrt();
        let xuv = if mass >= 0.7 { 1.0 } else { 1.0 + 8.0 * ((0.7 - mass) / 0.55).clamp(0.0, 1.0) };
        Ctx {
            name: sp.name.clone(),
            regime,
            mass,
            lum,
            teff,
            radius_sol,
            age,
            feh,
            a_snow,
            xuv,
        }
    }

    fn mass_me(&self) -> f64 {
        self.mass * SOLAR_TO_EARTH_MASS
    }
}

// ---------------------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tag {
    #[default]
    Planet,
    DwarfPlanet,
    Moon,
    DwarfMoon,
    Asteroid,
}

impl Tag {
    fn key(self) -> &'static str {
        match self {
            Tag::Planet => "Planet",
            Tag::DwarfPlanet => "DwarfPlanet",
            Tag::Moon => "Moon",
            Tag::DwarfMoon => "DwarfMoon",
            Tag::Asteroid => "Asteroid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Kind {
    /// Solid body (rock / iron / ice mixes), planets down to dwarf planets and moons.
    #[default]
    Solid,
    /// Hot stripped core (class Chthonia).
    Chthonian,
    /// Sub-Neptune / Neptune with an H/He envelope.
    Envelope,
    GasGiant,
    /// Asteroid-like small body.
    Small,
}

#[derive(Debug, Clone, Default)]
struct Atm {
    model: &'static str,
    height_km: f64,
    pressure_atm: f64,
    density: f64,
    greenhouse_k: f64,
    adiabat: f64,
    bright: f64,
    opacity: f64,
    sky_light: f64,
    hue: Option<f64>,
    saturation: Option<f64>,
    comp: Vec<(&'static str, f64)>,
}

#[derive(Debug, Clone, Default)]
struct Ocean {
    height_km: f64,
    comp: Vec<(&'static str, f64)>,
}

#[derive(Debug, Clone, Default)]
struct Clouds {
    height_km: f64,
    opacity: f64,
    coverage: f64,
    velocity_kmh: f64,
    main_freq: f64,
    main_octaves: u32,
    stripe_zones: f64,
    stripe_fluct: f64,
    stripe_twist: f64,
    modulate_bright: f64,
    day_ambient: f64,
}

/// A body's life, as its `Life` block: class ("Organic", "Exotic"), type ("Unicellular",
/// "Multicellular") and biome ("Marine", "Marine/Terrestrial").
#[derive(Debug, Clone, Default)]
pub struct Life {
    pub class: &'static str,
    pub kind: &'static str,
    pub biome: &'static str,
}

#[derive(Debug, Clone, Default)]
struct Rings {
    inner_km: f64,
    outer_km: f64,
    thickness_km: f64,
    opacity: f64,
    brightness: f64,
    density: f64,
}

#[derive(Debug, Clone, Default)]
struct Aurora {
    height_km: f64,
    top: [f64; 3],
    bottom: [f64; 3],
    north_lat: f64,
    north_lon: f64,
    south_lat: f64,
    south_lon: f64,
    radius_km: f64,
    width_km: f64,
    rings: u32,
    bright: f64,
}

#[derive(Debug, Clone, Default)]
struct GiantLook {
    surf_style: f64,
    tropic_lat: f64,
    cyclone_magn: f64,
    cyclone_freq: f64,
    cyclone_density: f64,
    cyclone_octaves: u32,
}

#[derive(Debug, Clone, Default)]
struct Body {
    name: String,
    parent: String,
    tag: Tag,
    kind: Kind,
    class: &'static str,
    mass_me: f64,
    radius_km: f64,
    /// Bulk water/ice mass fraction.
    wmf: f64,
    /// Core (iron) mass fraction of the rock part.
    cmf: f64,
    // Orbit (about the parent). Moons use AU too.
    a_au: f64,
    ecc: f64,
    inc: f64,
    node: f64,
    argp: f64,
    ma: f64,
    ref_plane: &'static str,
    /// Heliocentric distance for insolation (moons: their planet's a).
    helio_a: f64,
    orbit_period_s: f64,
    // Physical
    albedo_bond: f64,
    color: [f64; 3],
    t_eq: f64,
    t_surf: f64,
    rot_h: f64,
    obliq: f64,
    eq_node: f64,
    rot_offset: f64,
    locked: bool,
    oblateness: f64,
    surf_style: f64,
    randomize: [f64; 3],
    bump_height: f64,
    bump_offset: f64,
    ocean_cover: f64,
    ice_cover: f64,
    venus_like: bool,
    atm: Option<Atm>,
    ocean: Option<Ocean>,
    clouds: Option<Clouds>,
    life: Option<Life>,
    rings: Option<Rings>,
    aurora: Option<Aurora>,
    giant: Option<GiantLook>,
    /// SE's number of moons for a planet from SE's layout.
    se_moons: Option<u32>,
    /// SE's composition of a planet from SE's layout, written to `Interior` for the completion.
    se_comp: Option<[f32; 6]>,
}

impl Body {
    fn pressure(&self) -> f64 {
        self.atm.as_ref().map(|a| a.pressure_atm).unwrap_or(0.0)
    }

    fn gravity(&self) -> f64 {
        let r = self.radius_km * 1e3;
        if r <= 0.0 { 0.0 } else { G * self.mass_me * EARTH_MASS_KG / (r * r) }
    }

    fn vesc_kms(&self) -> f64 {
        let r = self.radius_km * 1e3;
        if r <= 0.0 { 0.0 } else { (2.0 * G * self.mass_me * EARTH_MASS_KG / r).sqrt() / 1e3 }
    }

    fn density_gcc(&self) -> f64 {
        let r = self.radius_km * 1e3;
        if r <= 0.0 {
            return 0.0;
        }
        self.mass_me * EARTH_MASS_KG / (4.0 / 3.0 * std::f64::consts::PI * r * r * r) / 1000.0
    }

    fn describe(&self) -> String {
        let cap = |s: String| {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => s,
            }
        };
        let size = size_prefix(self.class, self.mass_me);
        let bulk = self.class.to_lowercase();
        match self.kind {
            Kind::GasGiant | Kind::Envelope => {
                cap(format!("{} {}{}", temperature_class(self.t_eq), size, bulk))
            }
            Kind::Small => cap(format!("{} asteroid", temperature_class(self.t_surf))),
            _ => {
                let life = if self.life.is_some() { " with life" } else { "" };
                cap(format!(
                    "{} {} {} {}{}{}",
                    temperature_class(self.t_surf),
                    pressure_class(self.pressure()),
                    volatile_class(self.ocean_cover, self.ice_cover),
                    size,
                    bulk,
                    life
                ))
            }
        }
    }

    fn summary(&self) -> BodySummary {
        BodySummary {
            name: self.name.clone(),
            tag: self.tag.key(),
            parent: self.parent.clone(),
            class: self.class.to_string(),
            class_desc: self.describe(),
            mass_me: self.mass_me,
            radius_km: self.radius_km,
            a_au: self.a_au,
            ecc: self.ecc,
            helio_a_au: self.helio_a,
            t_eq_k: self.t_eq,
            t_surf_k: self.t_surf,
            pressure_atm: self.pressure(),
            albedo_bond: self.albedo_bond,
            tidally_locked: self.locked,
            has_atmosphere: self.atm.is_some(),
            has_ocean: self.ocean.is_some(),
            has_life: self.life.is_some(),
            life: self.life.clone(),
            has_rings: self.rings.is_some(),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Physics helpers
// ---------------------------------------------------------------------------------------

const AU_KM: f64 = METERS_PER_AU / 1000.0;
const SOLAR_TO_EARTH_MASS: f64 = SOLAR_MASS_KG / EARTH_MASS_KG;

/// Equilibrium temperature (K) with full heat redistribution.
fn t_eq(lum: f64, a_au: f64, ecc: f64, bond: f64) -> f64 {
    let t = 278.5 * lum.powf(0.25) / a_au.max(1e-6).sqrt() * (1.0 - bond).max(0.0).powf(0.25)
        * (1.0 - ecc * ecc).max(0.05).powf(-0.125);
    t.max(2.7)
}

fn orbital_period_s(a_au: f64, parent_mass_kg: f64) -> f64 {
    let a = a_au * METERS_PER_AU;
    std::f64::consts::TAU * (a * a * a / (G * parent_mass_kg.max(1.0))).sqrt()
}

fn normal_of(inc_deg: f64, node_deg: f64) -> DVec3 {
    let (i, o) = (inc_deg.to_radians(), node_deg.to_radians());
    DVec3::new(i.sin() * o.sin(), -i.sin() * o.cos(), i.cos())
}

/// (inclination, node) in degrees of a plane normal; inverse of [`normal_of`].
fn elements_of(n: DVec3) -> (f64, f64) {
    let n = n.normalize_or(DVec3::Z);
    let inc = n.z.clamp(-1.0, 1.0).acos().to_degrees();
    let node = n.x.atan2(-n.y).to_degrees().rem_euclid(360.0);
    (inc, node)
}

/// Tilt a plane normal by `delta_deg` toward azimuth `phi_deg`; returns (inc, node).
fn tilt_normal(inc: f64, node: f64, delta_deg: f64, phi_deg: f64) -> (f64, f64) {
    let n = normal_of(inc, node);
    let helper = if n.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let e1 = helper.cross(n).normalize();
    let e2 = n.cross(e1);
    let (d, p) = (delta_deg.to_radians(), phi_deg.to_radians());
    elements_of(n * d.cos() + (e1 * p.cos() + e2 * p.sin()) * d.sin())
}

fn true_anomaly_deg(m_deg: f64, e: f64) -> f64 {
    let m = m_deg.to_radians();
    (m + 2.0 * e * m.sin() + 1.25 * e * e * (2.0 * m).sin()).to_degrees()
}

/// Radius (km) of a solid body from mass, water-ice fraction and iron fraction.
fn solid_radius_km(mass_me: f64, wmf: f64, cmf: f64) -> f64 {
    if mass_me < 0.01 {
        let rho_rock = 3300.0 + 4000.0 * (cmf - 0.33);
        let inv = wmf / 1000.0 + (1.0 - wmf) / rho_rock;
        let rho = 1.0 / inv;
        let m = mass_me * EARTH_MASS_KG;
        (3.0 * m / (4.0 * std::f64::consts::PI * rho)).cbrt() / 1e3
    } else {
        let c = (1.0 - 0.27 * (cmf - 0.33)) * (1.0 + 0.75 * wmf);
        let p = if mass_me <= 1.0 { 0.30 } else { 0.27 };
        EARTH_RADIUS_KM * c * mass_me.powf(p)
    }
}

/// Radius (km) of a gas giant (M >= ~60 Earth masses), with hot-Jupiter inflation.
fn giant_radius_km(mass_me: f64, teq: f64) -> f64 {
    let m = mass_me;
    let r_e = if m < 95.2 {
        let (m0, r0) = (60.0, 0.867 * 60f64.powf(0.55));
        r0 + (9.45 - r0) * ((m / m0).ln() / (95.2f64 / m0).ln()).clamp(0.0, 1.0)
    } else if m < 317.8 {
        9.45 + (11.2 - 9.45) * (m / 95.2).ln() / (317.8f64 / 95.2).ln()
    } else {
        11.2 * (m / 317.8).powf(-0.04)
    };
    let infl = 1.0 + 0.25 * ((teq - 1000.0) / 1000.0).clamp(0.0, 1.0);
    r_e * infl * EARTH_RADIUS_KM
}

/// Radius (km) of an envelope planet (sub-Neptune / Neptune), Chen & Kipping style.
fn envelope_radius_km(mass_me: f64, rock_r_km: f64, jitter: f64) -> f64 {
    let r = 0.867 * mass_me.powf(0.55) * EARTH_RADIUS_KM * jitter;
    r.max(rock_r_km * 1.15)
}

fn mu_of(gas: &str) -> f64 {
    match gas {
        "N2" => 28.0,
        "O2" => 32.0,
        "CO2" => 44.0,
        "H2O" => 18.0,
        "Ar" => 40.0,
        "CH4" => 16.0,
        "He" => 4.0,
        "H2" => 2.0,
        "SO2" => 64.0,
        "NH3" => 17.0,
        "CO" => 28.0,
        "Cl2" => 71.0,
        "Ne" => 20.0,
        _ => 28.0,
    }
}

fn mean_mu(comp: &[(&'static str, f64)]) -> f64 {
    let tot: f64 = comp.iter().map(|c| c.1).sum();
    if tot <= 0.0 {
        return 28.0;
    }
    comp.iter().map(|c| mu_of(c.0) * c.1).sum::<f64>() / tot
}

/// Normalise a composition (percent) so the listed gases sum to 100, putting the remainder
/// into `filler` (which must be in the list).
fn fill_comp(mut comp: Vec<(&'static str, f64)>, filler: &'static str) -> Vec<(&'static str, f64)> {
    let others: f64 = comp.iter().filter(|c| c.0 != filler).map(|c| c.1).sum();
    let rest = (100.0 - others).max(0.0);
    for c in comp.iter_mut() {
        if c.0 == filler {
            c.1 = rest;
        }
    }
    comp.retain(|c| c.1 > 0.0);
    comp
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Build the `Atmosphere` block data for a given surface pressure and composition.
#[allow(clippy::too_many_arguments)]
fn make_atm(
    rng: &mut Rng,
    model: &'static str,
    comp: Vec<(&'static str, f64)>,
    p_atm: f64,
    greenhouse: f64,
    t_surf: f64,
    t_eq_k: f64,
    gravity: f64,
    adiabat: f64,
) -> Atm {
    let mu = mean_mu(&comp);
    let t_mean = 0.5 * (t_surf + t_eq_k);
    let h_scale_km = 8314.46 * t_mean / (mu * gravity.max(0.05)) / 1e3;
    let height = (h_scale_km * rng.range(8.0, 11.0)).clamp(8.0, 900.0);
    let density = p_atm * 101_325.0 * (mu / 1000.0) / (8.314_46 * t_surf.max(3.0));
    let lp = p_atm.max(1e-12).log10();
    // Visual strength: Pluto/Triton-like at 1e-5 atm up to Earth/Venus-like >= 1 atm.
    let opacity = (0.15 + (lp + 5.0) * 0.3).clamp(0.15, 1.0);
    let bright = if lp < -2.2 { lerp(0.5, 4.0, (lp + 8.0) / 5.8) } else { lerp(4.0, 10.0, (lp + 2.2) / 2.2) };
    let sky = if lp < -2.2 { lerp(0.01, 2.0, (lp + 8.0) / 5.8) } else { lerp(2.0, 3.0, (lp + 2.2) / 2.2) };
    Atm {
        model,
        height_km: height,
        pressure_atm: p_atm,
        density,
        greenhouse_k: greenhouse,
        adiabat,
        bright,
        opacity,
        sky_light: sky,
        hue: None,
        saturation: None,
        comp,
    }
}

// ---------------------------------------------------------------------------------------
// Planet generation
// ---------------------------------------------------------------------------------------

/// SE's class fields (luminosity class, type) of the star: from its spectral class when the
/// engine gives one, else from its kind, size and temperature.
fn star_class_bits(sp: &StarParams, c: &Ctx) -> (u32, u32) {
    if let Some(sc) = &sp.class {
        let (l, t, _) = crate::climate::star_class_bits(sc);
        return (l, t);
    }
    let t = c.teff;
    let letter = [30000.0, 10000.0, 7500.0, 6000.0, 5200.0, 3700.0, 2400.0, 1300.0, 550.0].iter().take_while(|&&lim| t < lim).count() as u32;
    let lum = match sp.kind {
        StarKind::WhiteDwarf => 9,
        StarKind::WolfRayet => 10,
        StarKind::NeutronStar => return (11, 13),
        StarKind::BlackHole => return (11, 14),
        StarKind::BrownDwarf => return (7, letter.max(7)),
        StarKind::Subdwarf => 8,
        _ if c.radius_sol > 6.0 => 5,
        _ => 7,
    };
    (lum, letter)
}

/// The star as SE's layout reads it.
fn se_star(sp: &StarParams, c: &Ctx) -> se_layout::SeStar {
    let (lum_code, letter) = star_class_bits(sp, c);
    se_layout::SeStar {
        mass_sol: c.mass,
        radius_km: c.radius_sol * crate::units::SOLAR_RADIUS_KM,
        teff: c.teff,
        lum_sol: c.lum,
        lum_code,
        letter,
        // Assumed: SE's octree draws the star's C/O and metallicity factor; solar values here.
        c_to_o: 0.55,
        met: 10f64.powf(c.feh) as f32,
    }
}

/// The root as SE's multiple-star generator reads it.
fn se_root(sp: &StarParams, c: &Ctx) -> se_multiple::Root {
    let class = sp.class.as_ref().map(|k| k.raw.clone()).unwrap_or_else(|| format!("{}V", crate::star_octree::ms_class_name(c.teff)));
    se_multiple::Root { star: se_star(sp, c), class, kind: sp.kind, age_gyr: c.age }
}

/// Planets (sorted outward) and belt asteroids of a single star from SE's layout
/// ([`se_layout`]), as bodies the port's physics then completes.
fn se_bodies(rng: &mut Rng, sp: &StarParams, c: &Ctx, seed: u32) -> (Vec<Body>, Vec<Body>, Vec<se_comets::SeComet>) {
    if big_black_hole(sp) {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let sys = se_layout::layout(&se_star(sp, c), seed);
    bodies_from(rng, c, &sys)
}

/// SE's planets and asteroids of one host as bodies, their orbits (and its comets') tilted onto a
/// random plane.
fn bodies_from(rng: &mut Rng, c: &Ctx, sys: &se_layout::SeSystem) -> (Vec<Body>, Vec<Body>, Vec<se_comets::SeComet>) {
    let sys_inc = rng.range(-1.0, 1.0).acos().to_degrees();
    let sys_node = rng.range(0.0, 360.0);
    let planets: Vec<Body> = sys
        .planets
        .iter()
        .map(|p| {
            let (inc, node) = tilt_normal(sys_inc, sys_node, p.inc_deg, p.node_deg);
            let rock = (p.comp[3] + p.comp[4] + p.comp[5]).max(1e-6);
            Body {
                tag: if p.dwarf { Tag::DwarfPlanet } else { Tag::Planet },
                kind: match p.class {
                    bulk::JUPITER => Kind::GasGiant,
                    bulk::NEPTUNE => Kind::Envelope,
                    _ => Kind::Solid,
                },
                class: BULK_NAMES[p.class.clamp(0, 10) as usize],
                mass_me: p.mass_me,
                radius_km: p.radius_km,
                wmf: p.comp[2] as f64,
                cmf: (p.comp[5] / rock) as f64,
                a_au: p.a_au,
                ecc: p.ecc,
                inc,
                node,
                argp: p.argp_deg.rem_euclid(360.0),
                ma: p.ma_deg.rem_euclid(360.0),
                ref_plane: "Ecliptic",
                helio_a: p.a_au,
                se_moons: Some(p.moons - p.debris),
                se_comp: Some(p.comp),
                ..Default::default()
            }
        })
        .collect();
    let asteroids = sys
        .asteroids
        .iter()
        .enumerate()
        .map(|(k, a)| {
            // SE's asteroid bodies (FUN_140502710) are not decoded: the port's sizes.
            let mut br = rng.fork(30_000 + k as u64);
            let (inc, node) = tilt_normal(sys_inc, sys_node, a.inc_deg, a.node_deg);
            let icy = a.a_au > c.a_snow;
            let radius = br.log_range(3.0, 250.0);
            let rho = if icy { br.range(1000.0, 2000.0) } else { br.range(1800.0, 3400.0) };
            Body {
                kind: Kind::Small,
                tag: Tag::Asteroid,
                a_au: a.a_au,
                ecc: a.ecc,
                inc,
                node,
                argp: a.argp_deg.rem_euclid(360.0),
                ma: a.ma_deg.rem_euclid(360.0),
                ref_plane: "Ecliptic",
                helio_a: a.a_au,
                radius_km: radius,
                wmf: if icy { 0.4 } else { 0.0 },
                mass_me: 4.0 / 3.0 * std::f64::consts::PI * (radius * 1e3).powi(3) * rho / EARTH_MASS_KG,
                ..Default::default()
            }
        })
        .collect();
    let comets = sys
        .comets
        .iter()
        .map(|k| {
            let (inc_deg, node_deg) = tilt_normal(sys_inc, sys_node, k.inc_deg, k.node_deg);
            se_comets::SeComet { inc_deg, node_deg, argp_deg: k.argp_deg.rem_euclid(360.0), ma_deg: k.ma_deg.rem_euclid(360.0), ..k.clone() }
        })
        .collect();
    (planets, asteroids, comets)
}

// ---------------------------------------------------------------------------------------
// Physical properties
// ---------------------------------------------------------------------------------------

fn physics_planet(rng: &mut Rng, c: &Ctx, b: &mut Body, star_mass_kg: f64) {
    b.orbit_period_s = orbital_period_s(b.a_au, star_mass_kg);
    match b.kind {
        Kind::GasGiant | Kind::Envelope => physics_giant(rng, c, b),
        _ => physics_solid(rng, c, b, c.regime == Regime::Neutron),
    }
    // Rotation about the star.
    let pole = None;
    assign_rotation(rng, c, b, star_mass_kg, b.a_au, pole);
    finish_visuals(rng, c, b);
}

fn physics_moon(rng: &mut Rng, c: &Ctx, m: &mut Body, planet: &Body) {
    let planet_mass_kg = planet.mass_me * EARTH_MASS_KG;
    m.orbit_period_s = orbital_period_s(m.a_au, planet_mass_kg);
    m.helio_a = planet.a_au;
    if m.kind == Kind::Small {
        small_body_physics(rng, c, m);
    } else {
        physics_solid(rng, c, m, c.regime == Regime::Neutron);
    }
    let pole = Some((planet.obliq, planet.eq_node));
    assign_rotation(rng, c, m, planet_mass_kg, m.a_au, pole);
    finish_visuals(rng, c, m);
}

fn physics_asteroid(rng: &mut Rng, c: &Ctx, a: &mut Body, star_mass_kg: f64) {
    a.orbit_period_s = orbital_period_s(a.a_au, star_mass_kg);
    small_body_physics(rng, c, a);
    assign_rotation(rng, c, a, star_mass_kg, a.a_au, None);
    finish_visuals(rng, c, a);
}

fn small_body_physics(rng: &mut Rng, c: &Ctx, b: &mut Body) {
    b.class = "Asteroid";
    if b.radius_km <= 0.0 {
        b.radius_km = solid_radius_km(b.mass_me, b.wmf, 0.3).max(0.5);
    }
    b.albedo_bond = if b.wmf > 0.2 { rng.range(0.03, 0.4) } else { rng.range(0.02, 0.2) };
    b.t_eq = t_eq(c.lum, b.helio_a.max(b.a_au.min(1e9)), b.ecc, b.albedo_bond);
    b.t_surf = b.t_eq;
    b.oblateness = rng.range(0.0, 0.3);
    let g = if b.wmf > 0.2 { [0.55, 0.55, 0.57] } else { [0.4, 0.38, 0.36] };
    let j = rng.range(0.85, 1.15);
    b.color = [g[0] * j, g[1] * j, g[2] * j];
}

fn physics_giant(rng: &mut Rng, c: &Ctx, b: &mut Body) {
    let t0 = t_eq(c.lum, b.a_au, b.ecc, 0.3);
    let gas = b.kind == Kind::GasGiant;
    // Class decides albedo, colour and atmosphere model (Sudarsky-like temperature classes).
    let (bond, col, hue, sat) = if gas {
        if t0 < 150.0 {
            (rng.range(0.30, 0.38), [0.85, 0.75, 0.6], rng.range(-0.02, 0.03), rng.range(0.7, 1.0))
        } else if t0 < 250.0 {
            (rng.range(0.5, 0.65), [0.93, 0.92, 0.88], 0.0, rng.range(0.4, 0.8))
        } else if t0 < 800.0 {
            (rng.range(0.08, 0.16), [0.3, 0.45, 0.78], rng.range(-0.08, -0.04), rng.range(0.8, 1.0))
        } else if t0 < 1400.0 {
            (rng.range(0.03, 0.08), [0.18, 0.15, 0.14], 0.0, 0.5)
        } else {
            (rng.range(0.2, 0.4), [0.55, 0.36, 0.3], 0.03, 0.9)
        }
    } else if t0 < 120.0 {
        let blue = rng.chance(0.5);
        (
            rng.range(0.28, 0.4),
            if blue { [0.35, 0.5, 0.85] } else { [0.72, 0.86, 0.9] },
            if blue { -0.053 } else { -0.081 },
            rng.range(0.8, 0.95),
        )
    } else if t0 < 500.0 {
        (rng.range(0.2, 0.35), [0.62, 0.74, 0.82], -0.06, 0.8)
    } else {
        (rng.range(0.05, 0.15), [0.3, 0.45, 0.75], -0.06, 0.9)
    };
    b.albedo_bond = bond;
    b.color = [col[0] * rng.range(0.93, 1.07), col[1] * rng.range(0.93, 1.07), col[2] * rng.range(0.93, 1.07)];
    b.t_eq = t_eq(c.lum, b.a_au, b.ecc, bond);
    b.t_surf = b.t_eq;
    if gas {
        b.class = "Jupiter";
        b.radius_km = giant_radius_km(b.mass_me, b.t_eq);
        b.tag = Tag::Planet;
    } else {
        b.class = "Neptune";
        // Hot Neptunes (T_eq > 1500 K and a small envelope) are photo-evaporated cores.
        let rock = solid_radius_km(b.mass_me.min(12.0), 0.1, 0.33);
        b.radius_km = envelope_radius_km(b.mass_me, rock, rng.range(0.9, 1.1));
        if b.t_eq > 1500.0 && b.mass_me < 20.0 && rng.chance(0.5) {
            b.kind = Kind::Chthonian;
            b.class = "Chthonia";
            b.cmf = rng.range(0.3, 0.4);
            b.wmf = 0.0;
            b.radius_km = solid_radius_km(b.mass_me, 0.0, b.cmf) * 1.05;
            b.albedo_bond = rng.range(0.05, 0.15);
            b.t_eq = t_eq(c.lum, b.a_au, b.ecc, b.albedo_bond);
            b.t_surf = b.t_eq;
            b.color = [0.25, 0.2, 0.18];
            physics_airless_extras(rng, b);
            return;
        }
    }
    b.randomize = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
    // Atmosphere (visual shell) and cloud bands.
    let g = b.gravity();
    let h2 = if gas { rng.range(0.86, 0.9) } else { rng.range(0.78, 0.82) };
    let he = if gas { 1.0 - h2 - 0.003 } else { rng.range(0.15, 0.19) };
    let ch4 = if gas { 0.3 } else { rng.range(1.0, 2.5) };
    let comp = fill_comp(vec![("H2", 0.0), ("He", he * 100.0), ("CH4", ch4)], "H2");
    let _ = h2;
    let mut atm = make_atm(rng, if gas { "Jupiter" } else { "Neptune" }, comp, 1.0, 0.0, b.t_surf, b.t_eq, g, 0.28);
    atm.height_km = (atm.height_km * 1.3).clamp(120.0, 900.0);
    atm.bright = 10.0;
    atm.opacity = 1.0;
    atm.sky_light = 1.0;
    atm.pressure_atm = 1e6;
    atm.density = 1000.0;
    atm.hue = Some(hue);
    atm.saturation = Some(sat);
    b.atm = Some(atm);
    b.clouds = Some(Clouds {
        height_km: rng.range(20.0, 40.0),
        opacity: 1.0,
        coverage: rng.range(0.3, 0.9),
        velocity_kmh: rng.range(100.0, 1400.0),
        main_freq: rng.range(0.3, 1.2),
        main_octaves: 8 + rng.below(5) as u32,
        stripe_zones: rng.range(1.0, 6.0),
        stripe_fluct: rng.range(0.2, 0.6),
        stripe_twist: rng.range(2.0, 12.0),
        modulate_bright: 1.0,
        day_ambient: 0.07,
    });
    b.giant = Some(GiantLook {
        surf_style: rng.range(0.0, 1.0),
        tropic_lat: rng.range(0.2, 0.5),
        cyclone_magn: rng.range(2.0, 12.0),
        cyclone_freq: rng.range(0.3, 1.0),
        cyclone_density: rng.range(0.1, 0.5),
        cyclone_octaves: 3 + rng.below(4) as u32,
    });
    b.aurora = if rng.chance(0.85) { Some(make_aurora(rng, b.radius_km, true)) } else { None };
    // Composition of the interior.
    let z = (22.0 / b.mass_me).min(0.9);
    b.wmf = z * 0.5;
    b.cmf = 0.3;
    // Rings.
    let p_ring = if gas { if b.mass_me > 60.0 && b.t_eq < 400.0 { 0.4 } else { 0.1 } } else { 0.25 };
    if rng.chance(p_ring) && b.a_au > 0.3 {
        let rho = b.density_gcc();
        let roche = 2.456 * b.radius_km * (rho.max(0.3) / 0.9).cbrt();
        let bright = rng.chance(0.25) && b.t_eq < 250.0;
        let inner = b.radius_km * rng.range(1.15, 1.5);
        let outer = if bright { (b.radius_km * rng.range(1.9, 2.4)).min(0.95 * roche) } else { (b.radius_km * rng.range(1.55, 1.95)).min(0.95 * roche) };
        if outer > inner * 1.1 {
            b.rings = Some(Rings {
                inner_km: inner,
                outer_km: outer,
                thickness_km: rng.range(0.05, 0.6),
                opacity: if bright { rng.range(0.8, 1.0) } else { rng.range(0.1, 0.4) },
                brightness: if bright { 4.0 } else { 1.0 },
                density: if bright { 2.0 } else { 0.5 },
            });
        }
    }
}

fn make_aurora(rng: &mut Rng, radius_km: f64, h2: bool) -> Aurora {
    let top = if h2 { [1.0, 1.0, 1.0] } else { [0.5, 0.5, 0.5] };
    let bottom = if h2 {
        [1.0, rng.range(0.0, 0.2), rng.range(0.4, 0.8)]
    } else {
        [0.0, 1.0, rng.range(0.0, 0.4)]
    };
    Aurora {
        height_km: if h2 { rng.range(800.0, 2500.0) } else { rng.range(80.0, 150.0) },
        top,
        bottom,
        north_lat: rng.range(75.0, 86.0),
        north_lon: rng.range(-180.0, 180.0),
        south_lat: -rng.range(62.0, 86.0),
        south_lon: rng.range(-180.0, 180.0),
        radius_km: radius_km * rng.range(0.25, 0.5),
        width_km: radius_km * rng.range(0.05, 0.15),
        rings: 2 + rng.below(2) as u32,
        bright: rng.range(0.3, 1.0),
    }
}

/// Composition, radius, class, atmosphere, ocean, life of a solid body (planet, dwarf
/// planet, big moon). `b.helio_a` is its heliocentric distance.
fn physics_solid(rng: &mut Rng, c: &Ctx, b: &mut Body, pulsar: bool) {
    let a_helio = b.helio_a.max(1e-4);
    let formed_icy = a_helio > c.a_snow * 1.1;
    // Composition.
    if b.wmf == 0.0 && b.cmf == 0.0 {
        b.cmf = (0.32 + 0.05 * c.feh + 0.06 * rng.normal()).clamp(0.15, 0.6);
        b.wmf = if formed_icy {
            rng.range(0.2, 0.5)
        } else if a_helio > c.a_snow * 0.6 {
            if rng.chance(0.1) { rng.range(0.05, 0.3) } else { rng.range(0.0, 0.01) }
        } else {
            rng.range(0.0, 0.003)
        };
        let p_fe = if a_helio / c.lum.sqrt().max(1e-3) < 0.3 { 0.25 } else { 0.05 };
        if b.wmf < 0.05 && rng.chance(p_fe) {
            b.cmf = rng.range(0.5, 0.68);
        }
    }
    let carbon = b.wmf < 0.05 && rng.chance(0.03);
    if b.kind != Kind::Chthonian {
        b.radius_km = solid_radius_km(b.mass_me, b.wmf, b.cmf) * if carbon { 0.97 } else { 1.0 };
    }
    b.class = if b.wmf >= 0.15 {
        "Aquaria"
    } else if carbon {
        "Carbonia"
    } else if b.cmf >= 0.5 {
        "Ferria"
    } else {
        "Terra"
    };
    b.randomize = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
    b.surf_style = rng.range(0.0, 1.0);
    let g = b.gravity();
    b.bump_height = (19.5 * (9.81 / g.max(0.05)).powf(0.8) * rng.range(0.7, 1.4)).clamp(1.5, 60.0);

    if pulsar {
        physics_airless_extras(rng, b);
        b.albedo_bond = rng.range(0.05, 0.15);
        b.t_eq = t_eq(c.lum, a_helio, b.ecc, b.albedo_bond);
        b.t_surf = b.t_eq;
        b.color = [0.42, 0.4, 0.38];
        return;
    }
    climate_solid(rng, c, b, a_helio, carbon);
    b.t_eq = t_eq(c.lum, a_helio, b.ecc, b.albedo_bond);
    // t_surf already includes greenhouse (set in climate_solid using the final albedo).
}

fn physics_airless_extras(rng: &mut Rng, b: &mut Body) {
    b.atm = None;
    b.ocean = None;
    b.clouds = None;
    b.life = None;
    b.randomize = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
    b.surf_style = rng.range(0.0, 1.0);
    b.bump_height = (19.5 * (9.81 / b.gravity().max(0.05)).powf(0.8)).clamp(1.5, 60.0);
}

fn comp_earthlike(rng: &mut Rng, o2: f64, co2: f64, ch4: f64) -> Vec<(&'static str, f64)> {
    fill_comp(
        vec![
            ("N2", 0.0),
            ("O2", o2),
            ("Ar", rng.range(0.3, 1.2)),
            ("H2O", rng.range(0.1, 1.5)),
            ("CO2", co2),
            ("CH4", ch4),
            ("Ne", 0.0018),
        ],
        "N2",
    )
}

fn comp_co2(rng: &mut Rng, thick: bool) -> Vec<(&'static str, f64)> {
    let co2 = if thick { rng.range(90.0, 97.0) } else { rng.range(92.0, 96.0) };
    fill_comp(
        vec![
            ("CO2", co2),
            ("N2", 0.0),
            ("Ar", rng.range(0.1, 2.0)),
            ("SO2", if thick { rng.range(0.001, 0.03) } else { 0.0 }),
            ("CO", rng.range(0.01, 0.1)),
        ],
        "N2",
    )
}

/// Atmosphere / ocean / life / clouds / albedo / surface temperature of a solid body.
fn climate_solid(rng: &mut Rng, c: &Ctx, b: &mut Body, a: f64, carbon: bool) {
    let s = c.lum / (a * a);
    let vesc = b.vesc_kms();
    let g = b.gravity();
    let m = b.mass_me;
    // Atmospheric retention: empirical "cosmic shoreline" (insolation ~ v_esc^4), calibrated so
    // Earth/Venus/Titan/Triton/Pluto keep an atmosphere and the Moon/Mercury/Callisto do not.
    let shore = (vesc / 7.0).powi(4) * 10f64.powf(0.25 * rng.normal());
    let retains = s * c.xuv < shore;
    let t_cold = t_eq(c.lum, a, b.ecc, 0.3);
    let icy_body = b.wmf >= 0.15;

    let mut atm_comp: Vec<(&'static str, f64)> = Vec::new();
    let mut model: &'static str = "Earth";
    let mut p = 0.0f64;
    let mut gh_coef = 0.0f64;
    let mut adiabat = 0.28;
    let mut albedo;
    let mut has_water = false;
    let mut global_ocean = false;
    let mut venus = false;
    let mut cloud_kind = 0u8; // 0 none, 1 venus, 2 water, 3 sparse dust/ice, 4 haze
    let mut life_ok = false;

    if !retains {
        albedo = if icy_body { rng.range(0.25, 0.7) } else if b.cmf >= 0.5 { rng.range(0.07, 0.13) } else { rng.range(0.06, 0.18) };
    } else if t_cold >= 262.0 {
        // Hot: runaway greenhouse / dry thick atmosphere.
        venus = true;
        if icy_body {
            atm_comp = fill_comp(vec![("H2O", rng.range(40.0, 80.0)), ("CO2", rng.range(10.0, 30.0)), ("N2", 0.0)], "N2");
            p = 10f64.powf(rng.range(2.0, 3.0));
        } else if rng.chance(0.05) {
            model = "Chlorine";
            atm_comp = fill_comp(vec![("Cl2", rng.range(40.0, 70.0)), ("CO2", rng.range(10.0, 30.0)), ("N2", 0.0)], "N2");
            p = 10f64.powf(rng.range(0.5, 1.8));
        } else {
            atm_comp = comp_co2(rng, true);
            p = 10f64.powf((1.4 + 0.5 * m.log10() + 0.45 * rng.normal()).clamp(-0.5, 2.3));
            model = if p > 20.0 { "Venus" } else if p > 3.0 { "Thick" } else { "Mars" };
        }
        gh_coef = 52.0;
        adiabat = 0.22;
        let bright_clouds = p > 20.0 && model != "Chlorine" && rng.chance(0.7);
        albedo = if bright_clouds { rng.range(0.7, 0.8) } else { rng.range(0.15, 0.3) };
        cloud_kind = if bright_clouds { 1 } else { 0 };
    } else if t_cold >= 208.0 {
        // Temperate: potentially habitable.
        if icy_body {
            has_water = true;
            global_ocean = true;
        } else {
            has_water = b.cmf < 0.5 && !carbon && rng.chance(if m >= 0.25 { 0.55 } else { 0.15 });
        }
        if has_water {
            p = 10f64.powf((0.0 + 0.3 * rng.normal()).clamp(-0.5, 0.7));
            if global_ocean {
                p *= rng.range(1.0, 3.0);
            }
            gh_coef = 33.0;
            albedo = rng.range(0.25, 0.36);
            cloud_kind = 2;
            life_ok = true;
        } else if rng.chance(0.5) {
            p = 10f64.powf((-0.2 + 0.9 * rng.normal()).clamp(-2.5, 1.5));
            atm_comp = comp_co2(rng, false);
            gh_coef = 52.0;
            adiabat = 0.22;
            albedo = rng.range(0.18, 0.3);
            cloud_kind = if p > 0.01 { 3 } else { 0 };
            model = if p < 0.3 { "Mars" } else { "Thick" };
        } else {
            p = 10f64.powf((-0.3 + 0.8 * rng.normal()).clamp(-2.5, 1.5));
            let co2 = rng.range(0.01, 3.0);
            atm_comp = comp_earthlike(rng, 0.0, co2, 0.0);
            gh_coef = 25.0;
            albedo = rng.range(0.18, 0.3);
            cloud_kind = if p > 0.05 { 3 } else { 0 };
            model = if p < 0.3 { "Thin" } else { "Earth" };
        }
    } else if t_cold >= 150.0 {
        // Cold.
        if icy_body && m >= 0.01 && rng.chance(0.4) {
            p = 10f64.powf(rng.range(-1.5, 0.8));
            atm_comp = comp_co2(rng, false);
            gh_coef = 40.0;
            adiabat = 0.22;
            albedo = rng.range(0.3, 0.6);
            model = "Thin";
            cloud_kind = 3;
        } else if m > 2.0 && rng.chance(0.5) {
            has_water = !icy_body && rng.chance(0.5);
            global_ocean = icy_body;
            p = 10f64.powf(rng.range(0.3, 1.1));
            atm_comp = fill_comp(vec![("CO2", rng.range(20.0, 80.0)), ("N2", 0.0), ("H2O", rng.range(0.1, 2.0))], "N2");
            gh_coef = 45.0;
            adiabat = 0.25;
            albedo = rng.range(0.25, 0.4);
            model = "Thick";
            cloud_kind = 2;
            has_water |= icy_body;
        } else {
            p = 10f64.powf((-2.2 + 0.6 * rng.normal()).clamp(-3.5, -1.0));
            atm_comp = comp_co2(rng, false);
            gh_coef = 52.0;
            adiabat = 0.22;
            albedo = if icy_body { rng.range(0.3, 0.6) } else { rng.range(0.15, 0.3) };
            model = "Mars";
            cloud_kind = if p > 0.002 { 3 } else { 0 };
        }
    } else {
        // Frozen: Titan / Triton / Pluto regime.
        if m >= 0.015 && rng.chance(0.45) {
            p = 10f64.powf(rng.range(0.0, 0.35));
            atm_comp = fill_comp(vec![("N2", 0.0), ("CH4", rng.range(1.5, 5.0)), ("H2", 0.1)], "N2");
            gh_coef = 10.0;
            adiabat = 0.28;
            albedo = rng.range(0.2, 0.3);
            model = "Titan";
            cloud_kind = 4;
        } else if m >= 0.0005 && rng.chance(0.55) {
            p = 10f64.powf(rng.range(-5.2, -4.4));
            atm_comp = fill_comp(vec![("N2", 0.0), ("CH4", rng.range(0.05, 0.5)), ("CO", rng.range(0.01, 0.1))], "N2");
            gh_coef = 0.0;
            albedo = rng.range(0.3, 0.7);
            model = "Pluto";
        } else {
            albedo = if icy_body { rng.range(0.25, 0.7) } else { rng.range(0.06, 0.2) };
        }
    }
    // A tiny pressure is "no atmosphere".
    if p < 1e-7 {
        p = 0.0;
    }

    // Greenhouse + surface temperature, with ice-albedo switching for water worlds.
    let gh_of = |p: f64, coef: f64, cold: f64| -> f64 {
        if p <= 0.0 {
            return 0.0;
        }
        let base = coef * p.sqrt();
        // Carbonate-silicate thermostat: extra CO2 greenhouse when a wet world gets cold.
        let extra = if coef == 33.0 { ((273.0 - (cold + base)) * 0.9).clamp(0.0, 70.0) } else { 0.0 };
        (base + extra).min(520.0)
    };
    let mut te = t_eq(c.lum, a, b.ecc, albedo);
    let mut gh = gh_of(p, gh_coef, te);
    let mut ts = te + gh;
    let mut frozen = false;
    if has_water && p > 0.0 {
        if ts < 258.0 {
            albedo = rng.range(0.5, 0.6);
            te = t_eq(c.lum, a, b.ecc, albedo);
            gh = gh_of(p, gh_coef, te);
            ts = te + gh;
            frozen = ts < 268.0;
        }
        let boil = 373.0 * p.max(0.01).powf(0.1);
        if ts > boil {
            // Oceans boil away: dry hot world.
            has_water = false;
            global_ocean = false;
            life_ok = false;
            venus = true;
            cloud_kind = 0;
        }
    }
    if icy_body && p == 0.0 && ts > 273.0 {
        // Airless but warm ice mixture would sublimate; treat as rocky-ish dark body.
        albedo = albedo.min(0.2);
    }
    b.albedo_bond = albedo;
    b.t_surf = ts.max(2.7);
    b.venus_like = venus && p > 20.0;

    // Atmosphere block.
    if p > 0.0 {
        if atm_comp.is_empty() {
            let co2 = rng.range(0.03, 0.5);
            atm_comp = comp_earthlike(rng, 0.0, co2, 0.0);
        }
        // Life.
        let ts_ok = (268.0..=332.0).contains(&b.t_surf);
        if life_ok && has_water && !frozen && ts_ok && (0.3..=6.0).contains(&p) && c.age > 0.5 {
            let p_life = if c.xuv > 3.0 { 0.2 } else { 0.35 };
            if rng.chance(p_life) {
                let multi = c.age > 2.0 && rng.chance(0.6);
                let exotic = rng.chance(0.06);
                b.life = Some(Life {
                    class: if exotic { "Exotic" } else { "Organic" },
                    kind: if multi { "Multicellular" } else { "Unicellular" },
                    biome: if !multi {
                        "Marine"
                    } else if global_ocean || rng.chance(0.2) {
                        "Marine"
                    } else {
                        "Marine/Terrestrial"
                    },
                });
                let o2 = if c.age > 2.4 && multi { rng.range(8.0, 32.0) } else if c.age > 2.4 { rng.range(0.5, 5.0) } else { rng.range(0.0, 0.5) };
                let co2 = rng.range(0.02, 0.3);
                let ch4 = if o2 < 1.0 { rng.range(0.1, 1.5) } else { 0.0 };
                atm_comp = comp_earthlike(rng, o2, co2, ch4);
                if exotic {
                    model = "Biogenic";
                }
            }
        }
        if has_water && b.life.is_none() && model == "Earth" && atm_comp.iter().all(|c| c.0 != "H2O") {
            let co2 = rng.range(0.02, 1.0);
            atm_comp = comp_earthlike(rng, 0.0, co2, 0.0);
        }
        let mut atm = make_atm(rng, model, atm_comp, p, gh, b.t_surf, te, g, adiabat);
        if model == "Titan" {
            atm.bright = 5.0;
            atm.sky_light = 1.667;
        }
        if model == "Mars" || model == "Thin" {
            atm.bright = atm.bright.min(4.0);
            atm.sky_light = atm.sky_light.min(2.0);
        }
        b.atm = Some(atm);
    }

    // Ocean.
    if has_water && !frozen && p > 0.006 && b.t_surf >= 268.0 {
        let (cover, level) = if global_ocean {
            (1.0, rng.range(1.2, 3.0))
        } else {
            let f = rng.range(0.2, 0.9);
            (f, 0.15 + 0.65 * f)
        };
        b.ocean_cover = cover;
        b.bump_offset = b.bump_height * level;
        let nacl = rng.range(1.5, 6.0);
        b.ocean = Some(Ocean { height_km: b.bump_offset, comp: vec![("H2O", 100.0 - nacl), ("NaCl", nacl)] });
    } else if frozen || (icy_body && b.t_surf < 200.0) {
        b.ice_cover = if frozen { rng.range(0.9, 1.0) } else { 1.0 };
    } else if has_water && b.t_surf < 268.0 {
        b.ice_cover = rng.range(0.4, 0.8);
    }

    // Clouds.
    b.clouds = match cloud_kind {
        1 => Some(Clouds { height_km: rng.range(55.0, 70.0), opacity: 1.0, coverage: 1.0, velocity_kmh: rng.range(300.0, 400.0), main_freq: rng.range(0.5, 1.5), main_octaves: 6, modulate_bright: 1.0, ..Default::default() }),
        2 if b.ocean.is_some() || frozen => Some(Clouds { height_km: rng.range(7.0, 12.0), opacity: rng.range(1.0, 1.35), coverage: rng.range(0.35, 0.75), velocity_kmh: rng.range(0.0, 60.0), main_freq: rng.range(0.4, 1.0), main_octaves: 6, modulate_bright: 1.333, ..Default::default() }),
        3 => Some(Clouds { height_km: rng.range(5.0, 20.0), opacity: rng.range(0.2, 0.6), coverage: rng.range(0.05, 0.25), velocity_kmh: rng.range(0.0, 80.0), main_freq: rng.range(0.5, 1.5), main_octaves: 5, modulate_bright: 1.0, ..Default::default() }),
        4 => Some(Clouds { height_km: rng.range(30.0, 60.0), opacity: rng.range(0.4, 0.8), coverage: rng.range(0.4, 0.7), velocity_kmh: rng.range(0.0, 40.0), main_freq: rng.range(0.4, 0.9), main_octaves: 5, modulate_bright: 1.0, ..Default::default() }),
        _ => None,
    };
    if b.atm.is_none() {
        b.clouds = None;
    }
    // Aurora on magnetised, atmosphered, fast rotators.
    if b.atm.is_some() && p > 0.05 && m > 0.3 && rng.chance(0.4) {
        b.aurora = Some(make_aurora(rng, b.radius_km, false));
    }

    // Colour.
    let col: [f64; 3] = if b.ocean.is_some() {
        if b.life.is_some() { [0.35, 0.52, 0.55] } else { [0.4, 0.5, 0.72] }
    } else if frozen || b.ice_cover > 0.5 {
        [0.85, 0.88, 0.92]
    } else if b.venus_like && b.clouds.is_some() {
        [0.93, 0.87, 0.66]
    } else if model == "Titan" {
        [0.85, 0.66, 0.38]
    } else if carbon {
        [0.13, 0.12, 0.11]
    } else if icy_body {
        [0.8, 0.82, 0.86]
    } else if b.atm.is_some() && p > 0.003 {
        [0.72, 0.52, 0.36]
    } else if b.cmf >= 0.5 {
        [0.5, 0.42, 0.38]
    } else {
        [0.46, 0.44, 0.41]
    };
    let j = rng.range(0.9, 1.1);
    b.color = [(col[0] * j).min(1.0), (col[1] * rng.range(0.92, 1.08)).min(1.0), (col[2] * rng.range(0.92, 1.08)).min(1.0)];
}

// ---------------------------------------------------------------------------------------
// Rotation
// ---------------------------------------------------------------------------------------

/// Spin, obliquity, tidal locking. `parent_mass_kg`/`a_au` describe the orbit the body is on;
/// `parent_pole` is the parent's (obliquity, EqAscendNode) for moons (their orbit plane is the
/// parent's equator), or `None` for bodies orbiting the star in the reference plane.
fn assign_rotation(rng: &mut Rng, c: &Ctx, b: &mut Body, parent_mass_kg: f64, a_au: f64, parent_pole: Option<(f64, f64)>) {
    let t_orb = orbital_period_s(a_au, parent_mass_kg);
    let r_m = b.radius_km * 1e3;
    let m_kg = b.mass_me * EARTH_MASS_KG;
    // Initial spin period (hours).
    let mut spin_h = match b.kind {
        Kind::GasGiant => rng.range(8.5, 18.0),
        Kind::Envelope => rng.range(13.0, 21.0),
        Kind::Small => rng.log_range(2.5, 40.0),
        _ => {
            if rng.chance(0.12) {
                rng.log_range(60.0, 1500.0)
            } else {
                rng.log_range(6.0, 60.0)
            }
        }
    };
    // Tidal locking time (Gladman et al. 1996 form; rigidity-dependent k2/Q).
    let (q, k2) = match b.kind {
        Kind::GasGiant | Kind::Envelope => (1e5, 0.3),
        _ => {
            // Love number of a homogeneous body of rigidity mu: k2 = 1.5 / (1 + 19 mu / (2 rho g R)).
            let mu = if b.wmf > 0.15 { 4e9 } else { 3e10 };
            let denom = (2.0 * b.density_gcc() * 1000.0 * b.gravity() * r_m).max(1e-9);
            (100.0, (1.5 / (1.0 + 19.0 * mu / denom)).clamp(1e-9, 0.5))
        }
    };
    let omega0 = std::f64::consts::TAU / (10.0 * 3600.0);
    let a_m = a_au * METERS_PER_AU;
    let t_lock_s = 0.4 * omega0 * a_m.powi(6) * m_kg * q / (3.0 * G * parent_mass_kg * parent_mass_kg * k2 * r_m.powi(3).max(1.0));
    let t_lock_yr = t_lock_s / SECONDS_PER_YEAR;
    let mut locked = t_lock_yr < c.age * 1e9;
    if b.kind == Kind::Small && b.tag == Tag::Asteroid && parent_pole.is_none() {
        locked = false;
    }
    let mut retro = false;
    if locked {
        if b.ecc > 0.2 && parent_pole.is_none() && rng.chance(0.7) {
            // 3:2 spin-orbit resonance (Mercury).
            spin_h = t_orb * (2.0 / 3.0) / 3600.0;
            locked = false;
            b.rot_offset = rng.range(0.0, 360.0);
        } else {
            spin_h = t_orb / 3600.0;
        }
    } else if b.venus_like && rng.chance(0.5) {
        spin_h = rng.log_range(1500.0, 6000.0);
        retro = rng.chance(0.7);
    } else if rng.chance(0.02) {
        retro = true;
    }
    b.locked = locked;
    b.rot_h = if retro { -spin_h } else { spin_h };

    // Obliquity relative to the orbit plane, then converted to the frame the engine uses.
    let eps = if locked {
        rng.range(0.0, 0.5)
    } else if parent_pole.is_some() && b.kind != Kind::Small {
        rng.rayleigh(2.0)
    } else {
        let u = rng.f64();
        if u < 0.7 {
            rng.rayleigh(12.0)
        } else if u < 0.9 {
            rng.range(20.0, 60.0)
        } else {
            rng.range(60.0, 140.0)
        }
    };
    let (base_inc, base_node) = match parent_pole {
        Some(p) => p,
        None => (b.inc, b.node),
    };
    let (psi, lam) = tilt_normal(base_inc, base_node, if retro { 180.0 - eps } else { eps }, rng.range(0.0, 360.0));
    b.obliq = psi;
    b.eq_node = lam;
    if locked {
        // Prime meridian toward the parent at the epoch (J2000).
        let nu = true_anomaly_deg(b.ma, b.ecc);
        b.rot_offset = (b.argp + nu + 180.0 + if parent_pole.is_some() { b.node - lam } else { 0.0 }).rem_euclid(360.0);
    } else if b.rot_offset == 0.0 {
        b.rot_offset = rng.range(0.0, 360.0);
    }
    // Oblateness from rotation (fluid-body scaling q = w^2 R^3 / GM).
    b.oblateness = if locked || b.kind == Kind::Small {
        if b.kind == Kind::Small { b.oblateness } else { 0.0 }
    } else {
        let w = std::f64::consts::TAU / (spin_h * 3600.0);
        let qv = w * w * r_m.powi(3) / (G * m_kg).max(1.0);
        let k = match b.kind {
            Kind::GasGiant | Kind::Envelope => 0.7,
            _ => 1.0,
        };
        (k * qv).min(0.35)
    };
}

fn finish_visuals(_rng: &mut Rng, _c: &Ctx, b: &mut Body) {
    if b.t_eq <= 0.0 {
        b.t_eq = 2.7;
    }
    if b.t_surf <= 0.0 {
        b.t_surf = b.t_eq;
    }
}

// ---------------------------------------------------------------------------------------
// Moons
// ---------------------------------------------------------------------------------------

fn gen_moons(rng: &mut Rng, c: &Ctx, p: &Body) -> Vec<Body> {
    let mut out = Vec::new();
    if p.tag == Tag::Asteroid || p.radius_km <= 0.0 {
        return out;
    }
    let mstar_me = c.mass_me();
    let r_hill_km = p.a_au * AU_KM * (1.0 - p.ecc) * (p.mass_me / (3.0 * mstar_me)).cbrt();
    let a_hi = 0.4 * r_hill_km;
    let rho_p = p.density_gcc().max(0.3);
    let mut a_lo = (1.26 * p.radius_km * (rho_p / 2.5).cbrt() * 1.3).max(1.8 * p.radius_km);
    if let Some(r) = &p.rings {
        a_lo = a_lo.max(r.outer_km * 1.2);
    }
    if a_hi <= a_lo * 1.5 {
        return out;
    }
    let giant = p.kind == Kind::GasGiant;
    let envelope = matches!(p.kind, Kind::Envelope | Kind::Chthonian);
    let close_in = p.a_au < 0.1;
    let tidal_f = if close_in { 0.15 } else { 1.0 };
    let (n_reg, n_small, n_irr): (usize, usize, usize);
    let mut big_q: Option<f64> = None;
    if giant {
        n_reg = rng.weighted(&[5.0, 5.0, 10.0, 15.0, 20.0, 20.0, 15.0, 10.0]);
        n_small = 0;
        n_irr = if p.a_au > 0.5 { rng.below(7) } else { 0 };
    } else if envelope {
        n_reg = rng.weighted(&[15.0, 15.0, 20.0, 20.0, 15.0, 10.0, 5.0]);
        n_small = 0;
        n_irr = if p.a_au > 0.5 { rng.below(5) } else { 0 };
    } else if p.tag == Tag::DwarfPlanet {
        n_reg = 0;
        n_small = if p.mass_me > 3e-4 { rng.weighted(&[60.0, 30.0, 8.0, 2.0]) } else { 0 };
        n_irr = 0;
        if n_small > 0 {
            big_q = Some(rng.log_range(0.003, 0.15));
        }
    } else if p.mass_me >= 0.1 {
        n_reg = 0;
        n_small = rng.weighted(&[50.0, 20.0, 20.0, 10.0]);
        n_irr = 0;
        if rng.chance(0.25) {
            big_q = Some(10f64.powf(rng.range(-3.0, -1.7)));
        }
    } else if p.mass_me >= 0.003 || p.se_moons.is_some() {
        n_reg = 0;
        n_small = rng.weighted(&[70.0, 20.0, 10.0]);
        n_irr = 0;
    } else {
        return out;
    }
    let scale = |n: usize, rng: &mut Rng| -> usize { (0..n).filter(|_| rng.chance(tidal_f)).count() };
    let (n_reg, n_small, n_irr) = match p.se_moons.map(|n| n as usize) {
        // SE's count (FUN_140500700); the port places them: a giant's regular moons up to its
        // own draw and the rest irregular, a solid body's all as small moons.
        Some(n) if giant || envelope => (n_reg.min(n), 0, n - n_reg.min(n)),
        Some(n) => {
            if n == 0 {
                big_q = None;
            }
            (0, n, 0)
        }
        None => (scale(n_reg, rng), scale(n_small, rng), scale(n_irr, rng)),
    };
    let icy_zone = p.a_au > c.a_snow * 1.1;
    let make = |rng: &mut Rng, mass: f64, a_km: f64, e: f64, inc: f64, _irregular: bool, icy: bool| -> Body {
        let wmf = if icy { rng.range(0.2, 0.5) } else { rng.range(0.0, 0.01) };
        let cmf = rng.range(0.2, 0.4);
        let mut b = Body {
            tag: Tag::Moon,
            kind: Kind::Solid,
            mass_me: mass,
            wmf,
            cmf,
            a_au: a_km / AU_KM,
            ecc: e,
            inc,
            node: rng.range(0.0, 360.0),
            argp: rng.range(0.0, 360.0),
            ma: rng.range(0.0, 360.0),
            ref_plane: "Equator",
            helio_a: p.a_au,
            ..Default::default()
        };
        b.radius_km = solid_radius_km(mass, wmf, cmf);
        if b.radius_km < 200.0 {
            b.tag = Tag::DwarfMoon;
            b.kind = Kind::Small;
            b.class = "Asteroid";
        }
        b
    };

    // Regular moons: resonance-style spacing outward from just outside the Roche/ring limit.
    if n_reg > 0 {
        let m_tot = p.mass_me * 10f64.powf(-3.7 + 0.3 * rng.normal());
        let mut w: Vec<f64> = (0..n_reg).map(|i| 10f64.powf(-0.35 * i as f64) * rng.range(0.6, 1.4)).collect();
        // shuffle which slot gets which weight
        for i in (1..w.len()).rev() {
            let j = rng.below(i + 1);
            w.swap(i, j);
        }
        let wsum: f64 = w.iter().sum();
        let mut a_km = a_lo * rng.range(1.0, 1.6);
        for (i, wi) in w.iter().enumerate() {
            if i > 0 {
                a_km *= rng.range(1.4, 2.1);
            }
            let e = rng.rayleigh(0.004).min(0.05);
            if a_km * (1.0 + e) > a_hi * 0.6 {
                break;
            }
            let mass = (m_tot * wi / wsum).max(1e-9);
            let icy = icy_zone && !(i < 2 && rng.chance(0.5));
            let inc = rng.rayleigh(0.5);
            out.push(make(rng, mass, a_km, e, inc, false, icy));
        }
    }
    // Small / large rocky-planet moons.
    let mut placed_big = false;
    if let Some(q) = big_q {
        let mass = q * p.mass_me;
        let lo = a_lo.max(8.0 * p.radius_km);
        let hi = (a_hi * 0.5).max(lo * 1.2).min(a_hi * 0.6);
        if hi > lo {
            let a_km = rng.log_range(lo, hi);
            let e = rng.rayleigh(0.03).min(0.2);
            if a_km * (1.0 + e) < a_hi * 0.6 {
                let icy = icy_zone;
                let inc = rng.rayleigh(3.0);
                out.push(make(rng, mass, a_km, e, inc, false, icy));
                placed_big = true;
            }
        }
    }
    let n_small = if big_q.is_some() && !placed_big { n_small.saturating_sub(0) } else { n_small };
    let mut a_km = a_lo * rng.range(1.0, 1.5);
    for i in 0..n_small {
        if i > 0 || placed_big {
            a_km *= rng.range(1.6, 2.6);
        }
        // skip over the big moon's neighbourhood
        let e = rng.rayleigh(0.01).min(0.1);
        if a_km * (1.0 + e) > a_hi * 0.5 {
            break;
        }
        if out.iter().any(|o| (o.a_au * AU_KM / a_km).ln().abs() < 0.3) {
            a_km *= 1.5;
            continue;
        }
        let radius = rng.log_range(3.0, 90.0);
        let rho = if icy_zone { rng.range(1000.0, 2000.0) } else { rng.range(1800.0, 3400.0) };
        let mass = 4.0 / 3.0 * std::f64::consts::PI * (radius * 1e3).powi(3) * rho / EARTH_MASS_KG;
        let inc = rng.rayleigh(1.5);
        let mut b = make(rng, mass, a_km, e, inc, false, icy_zone);
        b.radius_km = radius;
        b.tag = if radius >= 200.0 { Tag::Moon } else { Tag::DwarfMoon };
        b.kind = Kind::Small;
        b.class = "Asteroid";
        out.push(b);
    }
    // Irregular captured satellites: far out, eccentric, inclined or retrograde.
    let reg_out = out.iter().map(|o| o.a_au * AU_KM * (1.0 + o.ecc)).fold(0.0f64, f64::max);
    for _ in 0..n_irr {
        let e = rng.range(0.1, 0.4);
        let lo = (0.08 * r_hill_km).max(reg_out * 1.3 / (1.0 - e)).max(a_lo / (1.0 - e));
        let hi = 0.3 * r_hill_km;
        if hi <= lo * 1.1 {
            continue;
        }
        let a_km = rng.log_range(lo, hi);
        if a_km * (1.0 + e) > 0.45 * r_hill_km {
            continue;
        }
        let radius = rng.log_range(2.0, 60.0);
        let rho = if icy_zone { rng.range(1200.0, 2200.0) } else { rng.range(2000.0, 3200.0) };
        let mass = 4.0 / 3.0 * std::f64::consts::PI * (radius * 1e3).powi(3) * rho / EARTH_MASS_KG;
        let inc = if rng.chance(0.5) { rng.range(20.0, 60.0) } else { rng.range(120.0, 160.0) };
        let mut b = make(rng, mass, a_km, e, inc, true, icy_zone);
        b.radius_km = radius;
        b.tag = if radius >= 200.0 { Tag::Moon } else { Tag::DwarfMoon };
        b.kind = Kind::Small;
        b.class = "Asteroid";
        out.push(b);
    }
    out.sort_by(|a, b| a.a_au.total_cmp(&b.a_au));
    // Names: ".N" for the larger (round) moons, ".SN" for small irregular/captured ones.
    let (mut n_big, mut n_s) = (0, 0);
    for m in out.iter_mut() {
        m.parent = p.name.clone();
        if m.tag == Tag::Moon {
            n_big += 1;
            m.name = format!("{}.{}", p.name, n_big);
        } else {
            n_s += 1;
            m.name = format!("{}.S{}", p.name, n_s);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// .sc emission
// ---------------------------------------------------------------------------------------

/// Compact number: ~6 significant digits, exponent form when very small/large.
fn num(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return "0".to_string();
    }
    let ax = x.abs();
    if !(1e-3..1e7).contains(&ax) {
        let s = format!("{:.6e}", x);
        // trim mantissa zeros: 1.500000e-5 -> 1.5e-5
        if let Some((m, e)) = s.split_once('e') {
            let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
            return format!("{m}e{e}");
        }
        return s;
    }
    let decimals = (5 - ax.log10().floor() as i32).clamp(0, 9) as usize;
    let s = format!("{:.*}", decimals, x);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

fn vec3(v: [f64; 3]) -> String {
    format!("( {:.3} {:.3} {:.3} )", v[0], v[1], v[2])
}

fn comp_block(out: &mut String, indent: &str, comp: &[(&'static str, f64)]) {
    let _ = writeln!(out, "{indent}Composition // values in percent\n{indent}{{");
    for (g, v) in comp {
        let _ = writeln!(out, "{indent}\t{g} {}", num(*v));
    }
    let _ = writeln!(out, "{indent}}}");
}

/// Planets and moons are written as SE's system generator leaves them: class, mass, radius, orbit,
/// rotation, life, rings and aurora. Like any catalog body without them, SE's completion chain then
/// gives them their composition, albedo, tidal heat, volcanism, atmosphere, ocean, clouds and
/// temperature (`climate::complete`), and SE's surface generator their surface (`surface_gen`).
/// Asteroids (which that chain skips) keep the designed blocks.
fn emit_body(out: &mut String, b: &Body) {
    let _ = writeln!(out, "\n// {} | Teq {:.0} K, Tsurf {:.0} K{}", b.describe(), b.t_eq, b.t_surf, if b.locked { ", tidally locked" } else { "" });
    let _ = writeln!(out, "{} \"{}\"\n{{", b.tag.key(), b.name);
    let _ = writeln!(out, "\tParentBody     \"{}\"", b.parent);
    let _ = writeln!(out, "\tClass          \"{}\"", b.class);
    let _ = writeln!(out, "\tMass           {}\t// Earth masses", num(b.mass_me));
    let _ = writeln!(out, "\tRadius         {}\t// km", num(b.radius_km));
    if b.oblateness > 1e-7 {
        let _ = writeln!(out, "\tOblateness     {}", num(b.oblateness));
    }
    let se_completes = b.tag != Tag::Asteroid;
    if !se_completes {
        let _ = writeln!(out, "\tColor          {}", vec3(b.color));
        let _ = writeln!(out, "\tAlbedoBond     {}", num(b.albedo_bond));
        let geom = (b.albedo_bond * if b.atm.is_some() { 1.2 } else { 1.4 }).min(1.0);
        let _ = writeln!(out, "\tAlbedoGeom     {}", num(geom));
    }
    let _ = writeln!(out, "\tRotationPeriod {}\t// hours", num(b.rot_h));
    let _ = writeln!(out, "\tObliquity      {}", num(b.obliq));
    let _ = writeln!(out, "\tEqAscendNode   {}", num(b.eq_node));
    let _ = writeln!(out, "\tRotationOffset {}", num(b.rot_offset));

    // Surface (procedural parameters only; no maps).
    match (&b.giant, b.kind) {
        _ if se_completes => {}
        (Some(g), _) => {
            let _ = writeln!(out, "\tSurface\n\t{{");
            let _ = writeln!(out, "\t\tSurfStyle       {}", num(g.surf_style));
            let _ = writeln!(out, "\t\tRandomize       {}", vec3(b.randomize));
            let _ = writeln!(out, "\t\ttropicLatitude  {}", num(g.tropic_lat));
            if let Some(cl) = &b.clouds {
                let _ = writeln!(out, "\t\tstripeZones     {}", num(cl.stripe_zones));
                let _ = writeln!(out, "\t\tstripeFluct     {}", num(cl.stripe_fluct));
                let _ = writeln!(out, "\t\tstripeTwist     {}", num(cl.stripe_twist));
            }
            let _ = writeln!(out, "\t\tcycloneMagn     {}", num(g.cyclone_magn));
            let _ = writeln!(out, "\t\tcycloneFreq     {}", num(g.cyclone_freq));
            let _ = writeln!(out, "\t\tcycloneDensity  {}", num(g.cyclone_density));
            let _ = writeln!(out, "\t\tcycloneOctaves  {}", g.cyclone_octaves);
            let _ = writeln!(out, "\t\tModulateBright  0\n\t}}");
        }
        _ if b.kind != Kind::Small => {
            let _ = writeln!(out, "\tSurface\n\t{{");
            let _ = writeln!(out, "\t\tSurfStyle       {}", num(b.surf_style));
            let _ = writeln!(out, "\t\tRandomize       {}", vec3(b.randomize));
            let _ = writeln!(out, "\t\tBumpHeight      {}\t// km, total terrain span", num(b.bump_height));
            if b.ocean.is_some() {
                let _ = writeln!(out, "\t\tBumpOffset      {}\t// km below sea level", num(b.bump_offset));
                let _ = writeln!(out, "\t\tSpecBrightWater 0.85");
                let _ = writeln!(out, "\t\tcolorSea       (0.012 0.03 0.06 1.00)");
                let _ = writeln!(out, "\t\tcolorShelf     (0.15 0.48 0.46 1.00)");
            }
            let _ = writeln!(out, "\t}}");
        }
        _ => {}
    }

    // Interior composition (mass fractions in percent).
    if let Some(c) = &b.se_comp {
        let _ = writeln!(out, "\tInterior\n\t{{");
        let _ = writeln!(out, "\t\tComposition // mass fractions, values in percent\n\t\t{{");
        for (k, x) in crate::climate::COMPOSITION_KEYS.iter().zip(c) {
            if *x > 0.0 {
                let _ = writeln!(out, "\t\t\t{} {}", k, num(*x as f64 * 100.0));
            }
        }
        let _ = writeln!(out, "\t\t}}\n\t}}");
    }
    if b.kind != Kind::Small && !se_completes {
        let _ = writeln!(out, "\tInterior\n\t{{");
        let _ = writeln!(out, "\t\tComposition // mass fractions, values in percent\n\t\t{{");
        if matches!(b.kind, Kind::GasGiant | Kind::Envelope) {
            let z = (22.0 / b.mass_me).min(0.9);
            let _ = writeln!(out, "\t\t\tHydrogen {}", num((1.0 - z) * 74.0));
            let _ = writeln!(out, "\t\t\tHelium {}", num((1.0 - z) * 26.0));
            let _ = writeln!(out, "\t\t\tSilicates {}", num(z * 40.0));
            let _ = writeln!(out, "\t\t\tMetals {}", num(z * 15.0));
            let _ = writeln!(out, "\t\t\tIces {}", num(z * 45.0));
        } else {
            let rock = 1.0 - b.wmf;
            let _ = writeln!(out, "\t\t\tIces {}", num(b.wmf * 100.0));
            let _ = writeln!(out, "\t\t\tSilicates {}", num(rock * (1.0 - b.cmf) * 100.0));
            let _ = writeln!(out, "\t\t\tMetals {}", num(rock * b.cmf * 100.0));
        }
        let _ = writeln!(out, "\t\t}}\n\t}}");
    }

    if let Some(o) = b.ocean.as_ref().filter(|_| !se_completes) {
        let _ = writeln!(out, "\tOcean\n\t{{");
        let _ = writeln!(out, "\t\tHeight {}", num(o.height_km));
        comp_block(out, "\t\t", &o.comp);
        let _ = writeln!(out, "\t}}");
    }
    if let Some(cl) = b.clouds.as_ref().filter(|_| !se_completes) {
        let _ = writeln!(out, "\tClouds\n\t{{");
        let _ = writeln!(out, "\t\tHeight          {}", num(cl.height_km));
        let _ = writeln!(out, "\t\tVelocity        {}", num(cl.velocity_kmh));
        let _ = writeln!(out, "\t\tOpacity         {}", num(cl.opacity));
        let _ = writeln!(out, "\t\tCoverage        {}", num(cl.coverage));
        let _ = writeln!(out, "\t\tModulateBright  {}", num(cl.modulate_bright));
        let _ = writeln!(out, "\t\tmainFreq        {}", num(cl.main_freq));
        let _ = writeln!(out, "\t\tmainOctaves     {}", cl.main_octaves);
        if cl.stripe_zones > 0.0 {
            let _ = writeln!(out, "\t\tDayAmbient      {}", num(cl.day_ambient));
            let _ = writeln!(out, "\t\tstripeZones     {}", num(cl.stripe_zones));
            let _ = writeln!(out, "\t\tstripeFluct     {}", num(cl.stripe_fluct));
            let _ = writeln!(out, "\t\tstripeTwist     {}", num(cl.stripe_twist));
        }
        let _ = writeln!(out, "\t}}");
    }
    if let Some(a) = b.atm.as_ref().filter(|_| !se_completes) {
        let _ = writeln!(out, "\tAtmosphere\n\t{{");
        let _ = writeln!(out, "\t\tModel          \"{}\"", a.model);
        let _ = writeln!(out, "\t\tHeight          {}\t// km", num(a.height_km));
        let _ = writeln!(out, "\t\tPressure        {}\t// atm", num(a.pressure_atm));
        let _ = writeln!(out, "\t\tDensity         {}\t// kg/m^3", num(a.density));
        let _ = writeln!(out, "\t\tGreenhouse      {}\t// K", num(a.greenhouse_k));
        let _ = writeln!(out, "\t\tAdiabat         {}", num(a.adiabat));
        let _ = writeln!(out, "\t\tBright          {}", num(a.bright));
        let _ = writeln!(out, "\t\tOpacity         {}", num(a.opacity));
        let _ = writeln!(out, "\t\tSkyLight        {}", num(a.sky_light));
        if let Some(h) = a.hue {
            let _ = writeln!(out, "\t\tHue             {}", num(h));
        }
        if let Some(s) = a.saturation {
            let _ = writeln!(out, "\t\tSaturation      {}", num(s));
        }
        comp_block(out, "\t\t", &a.comp);
        let _ = writeln!(out, "\t}}");
    }
    if let Some(l) = &b.life {
        let _ = writeln!(out, "\tLife\n\t{{\n\t\tClass   \"{}\"\n\t\tType    \"{}\"\n\t\tBiome   \"{}\"\n\t}}", l.class, l.kind, l.biome);
    }
    if let Some(a) = &b.aurora {
        let _ = writeln!(out, "\tAurora\n\t{{");
        let _ = writeln!(out, "\t\tHeight       {}", num(a.height_km));
        let _ = writeln!(out, "\t\tTopColor     {}", vec3(a.top));
        let _ = writeln!(out, "\t\tBottomColor  {}", vec3(a.bottom));
        let _ = writeln!(out, "\t\tNorthLat     {}\n\t\tNorthLon     {}", num(a.north_lat), num(a.north_lon));
        let _ = writeln!(out, "\t\tNorthRadius  {}\n\t\tNorthWidth   {}", num(a.radius_km), num(a.width_km));
        let _ = writeln!(out, "\t\tNorthRings   {}\n\t\tNorthBright  {}", a.rings, num(a.bright));
        let _ = writeln!(out, "\t\tSouthLat     {}\n\t\tSouthLon     {}", num(a.south_lat), num(a.south_lon));
        let _ = writeln!(out, "\t\tSouthRadius  {}\n\t\tSouthWidth   {}", num(a.radius_km), num(a.width_km));
        let _ = writeln!(out, "\t\tSouthRings   {}\n\t\tSouthBright  {}", a.rings, num(a.bright));
        let _ = writeln!(out, "\t}}");
    }
    if let Some(r) = &b.rings {
        let _ = writeln!(out, "\tRings\n\t{{");
        let _ = writeln!(out, "\t\tInnerRadius     {}\t// km", num(r.inner_km));
        let _ = writeln!(out, "\t\tOuterRadius     {}\t// km", num(r.outer_km));
        let _ = writeln!(out, "\t\tThickness       {}", num(r.thickness_km));
        let _ = writeln!(out, "\t\tBrightness      {}", num(r.brightness));
        let _ = writeln!(out, "\t\tFrontBright     {}", num(r.brightness * 1.25));
        let _ = writeln!(out, "\t\tBackBright      {}", num(r.brightness * 0.6));
        let _ = writeln!(out, "\t\tDensity         {}", num(r.density));
        let _ = writeln!(out, "\t\tOpacity         {}", num(r.opacity));
        let _ = writeln!(out, "\t\tSelfShadow      1\n\t\tPlanetShadow    1\n\t\tHapke           1");
        let _ = writeln!(out, "\t}}");
    } else if matches!(b.tag, Tag::Planet | Tag::DwarfPlanet) {
        let _ = writeln!(out, "\tNoRings        true");
    }
    if !se_completes {
        let _ = writeln!(out, "\tNoAtmosphere   true\n\tNoOcean        true\n\tNoClouds       true");
    }
    if b.aurora.is_none() {
        let _ = writeln!(out, "\tNoAurora       true");
    }
    let _ = writeln!(out, "\tNoCometTail    true");

    let _ = writeln!(out, "\tOrbit\n\t{{");
    let _ = writeln!(out, "\t\tRefPlane        \"{}\"", b.ref_plane);
    let _ = writeln!(out, "\t\tSemiMajorAxis   {}\t// AU", num(b.a_au));
    let _ = writeln!(out, "\t\tEccentricity    {}", num(b.ecc));
    let _ = writeln!(out, "\t\tInclination     {}", num(b.inc));
    let _ = writeln!(out, "\t\tAscendingNode   {}", num(b.node));
    let _ = writeln!(out, "\t\tArgOfPericenter {}", num(b.argp));
    let _ = writeln!(out, "\t\tMeanAnomaly     {}", num(b.ma));
    let _ = writeln!(out, "\t}}\n}}");
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{BodyTag, Catalog, ObjectRef};

    fn sun_like(name: &str) -> StarParams {
        StarParams {
            name: name.to_string(),
            mass_sol: 1.0,
            lum_bol: 1.0,
            teff: 5778.0,
            radius_sol: 1.0,
            age_gyr: 4.6,
            feh: 0.0,
            kind: StarKind::Normal,
            class: None,
        }
    }

    fn star(mass: f64, kind: StarKind, r: f64, age: f64) -> StarParams {
        let lum = if kind == StarKind::Normal && r < 3.0 { mass.powf(3.5) } else { (r * r * 1.0).max(1e-4) };
        StarParams {
            name: format!("RS 0-1-2-{}", (mass * 100.0) as u32),
            mass_sol: mass,
            lum_bol: lum,
            teff: 5000.0,
            radius_sol: r,
            age_gyr: age,
            feh: 0.0,
            kind,
            class: None,
        }
    }

    #[test]
    fn deterministic() {
        let s = sun_like("RS 8514-491-6-169551-68");
        let a = generate_system_sc(&s, 12345);
        let b = generate_system_sc(&s, 12345);
        assert_eq!(a, b);
        assert_ne!(a, generate_system_sc(&s, 12346));
        let mut s2 = s.clone();
        s2.name = "RS other".into();
        assert_ne!(a, generate_system_sc(&s2, 12345));
    }

    #[test]
    fn rng_basics() {
        let mut r = Rng::new(1);
        let v: Vec<f64> = (0..1000).map(|_| r.f64()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = v.iter().sum::<f64>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05);
        let f1 = Rng::new(9).fork(3).next_u64();
        let mut base = Rng::new(9);
        base.next_u64();
        assert_eq!(f1, base.fork(3).next_u64(), "fork must not depend on position");
    }

    #[test]
    fn star_names() {
        let n = procedural_star_name("8514-491", [3, -4, 5], 6, 68);
        assert!(n.starts_with("RS 8514-491-6-") && n.ends_with("-68"), "{n}");
        assert_eq!(n, procedural_star_name("RG 8514-491", [3, -4, 5], 6, 68));
        assert_ne!(cell_node_id([1, 2, 3]), cell_node_id([3, 2, 1]));
        assert_eq!(planet_name("RS 8514-491-6-169551-68", 3), "RS 8514-491-6-169551-68 3");
        assert_eq!(planet_name("RS 0-1-3-1102-3821-2-11-619 A", 4), "RS 0-1-3-1102-3821-2-11-619 A4");
    }

    /// A catalog with `s` (a barycentre when the system is multiple) and its generated system.
    fn load_generated(s: &StarParams, g: &GeneratedSystem) -> Catalog {
        let mut cat = Catalog::new();
        let root = match g.root_mass_sol {
            Some(m) => format!("Barycenter \"{}\" {{ RA 1 Dec 2 Dist 10 MassSol {m} }}", s.name),
            None => format!("Star \"{}\" {{ RA 1 Dec 2 Dist 10 Class \"G2V\" MassSol {} }}", s.name, s.mass_sol),
        };
        cat.load_sc(&root, "star.sc");
        cat.load_sc(&g.sc, "gen.sc");
        cat.finalize();
        cat
    }

    #[test]
    fn parses_and_resolves() {
        let s = sun_like("RS 8514-491-6-169551-68");
        let mut found_any = false;
        for seed in 0..40u64 {
            let gen_ = generate_system(&s, seed);
            let cat = load_generated(&s, &gen_);
            assert!(cat.warnings.iter().all(|w| !w.contains("no ParentBody")), "{:?}", cat.warnings);
            let all = cat.system_members(&s.name);
            let stars = all.iter().filter(|m| matches!(m, ObjectRef::Star(i) if !cat.stars[*i as usize].barycenter)).count();
            assert_eq!(stars, gen_.stars.len(), "seed {seed}");
            let members: Vec<ObjectRef> = all.into_iter().filter(|m| matches!(m, ObjectRef::Body(_))).collect();
            assert_eq!(members.len(), gen_.bodies.len(), "seed {seed}");
            if !members.is_empty() {
                found_any = true;
            }
            for m in &members {
                let ObjectRef::Body(i) = *m else { panic!("non-body member") };
                let b = &cat.bodies[i as usize];
                assert!(b.radius_km > 0.0, "{} radius", b.names[0]);
                assert!(b.mass_kg > 0.0, "{} mass", b.names[0]);
                let o = b.orbit.as_ref().unwrap_or_else(|| panic!("{} has no orbit", b.names[0]));
                assert!(o.period > 0.0 && o.semi_major_axis > 0.0, "{} period", b.names[0]);
                assert!(o.eccentricity < 1.0);
                // ParentBody chain resolves to the star.
                let mut cur = b.parent.clone();
                let mut depth = 0;
                loop {
                    match cat.find(&cur) {
                        Some(ObjectRef::Star(_)) => break,
                        Some(ObjectRef::Body(j)) => cur = cat.bodies[j as usize].parent.clone(),
                        other => panic!("parent chain broken at {cur}: {other:?}"),
                    }
                    depth += 1;
                    assert!(depth < 4);
                }
                assert!(matches!(
                    b.tag,
                    BodyTag::Planet | BodyTag::DwarfPlanet | BodyTag::Moon | BodyTag::DwarfMoon | BodyTag::Asteroid | BodyTag::Comet
                ));
            }
        }
        assert!(found_any);
    }

    #[test]
    fn statistics_sun_like() {
        // SE's architecture (se_layout): 5-14 slots, geometric spacing out to the sphere of
        // influence, gas giants ~12% and ice giants ~25% of planets.
        let mut counts = Vec::new();
        let (mut moons, mut giants, mut neptunes, mut dwarfs, mut planets_total, mut belts) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        for seed in 0..200u64 {
            let s = sun_like(&format!("RS 0-8-1-{seed}"));
            let g = generate_system(&s, seed.wrapping_mul(7919));
            let planets: Vec<&BodySummary> =
                g.bodies.iter().filter(|b| b.tag == "Planet" || b.tag == "DwarfPlanet").collect();
            let mut hosts: Vec<&str> = planets.iter().map(|p| p.parent.as_str()).collect();
            hosts.dedup();
            for h in hosts {
                let ps: Vec<&&BodySummary> = planets.iter().filter(|p| p.parent == h).collect();
                if g.stars.is_empty() {
                    counts.push(ps.len());
                }
                assert!((1..=14).contains(&ps.len()), "seed {seed}: {} planets of {h}", ps.len());
                // Outward by SE's geometric steps (SE's guard against crossing orbits mixes AU
                // and parsecs, so orbits may cross as in SE).
                for w in ps.windows(2) {
                    assert!(w[0].a_au * 1.19 < w[1].a_au, "seed {seed}: {} and {} too close", w[0].name, w[1].name);
                }
            }
            for p in &planets {
                planets_total += 1;
                // Circumbinary planets reach out to 1000 AU around wide binaries.
                assert!(p.a_au > 0.005 && p.a_au < 3000.0, "{} a={}", p.name, p.a_au);
                assert!(p.ecc >= 0.0 && p.ecc < 0.8);
                assert!(p.mass_me > 1e-5 && p.mass_me < 5000.0, "{} M {}", p.name, p.mass_me);
                assert!(p.radius_km > 100.0 && p.radius_km < 130_000.0, "{} R {}", p.name, p.radius_km);
                assert_eq!(p.tag == "DwarfPlanet", p.radius_km <= 2000.0, "{} R {}", p.name, p.radius_km);
                giants += (p.class == "Jupiter") as usize;
                neptunes += (p.class == "Neptune") as usize;
                dwarfs += (p.tag == "DwarfPlanet") as usize;
            }
            // Moons: outside the planet, Hill-stable (about a one-solar-mass single star).
            for m in g.bodies.iter().filter(|b| g.stars.is_empty() && (b.tag == "Moon" || b.tag == "DwarfMoon")) {
                moons += 1;
                let host = g.bodies.iter().find(|b| b.name == m.parent).expect("moon host");
                let rh_km = host.a_au * AU_KM * (1.0 - host.ecc) * (host.mass_me / (3.0 * SOLAR_TO_EARTH_MASS)).cbrt();
                assert!(m.a_au * AU_KM * (1.0 + m.ecc) < 0.5 * rh_km, "seed {seed}: {} beyond 0.5 RH", m.name);
                assert!(m.a_au * AU_KM * (1.0 - m.ecc) > host.radius_km * 1.5, "{} inside its planet", m.name);
            }
            // Belts: 100-1000 asteroids in all.
            let n_ast = g.bodies.iter().filter(|b| b.tag == "Asteroid").count();
            assert!(n_ast <= 1000);
            if n_ast > 0 {
                belts += 1;
                assert!(n_ast >= 100);
            }
        }
        let mean = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
        let (fg, fn_) = (giants as f64 / planets_total as f64, neptunes as f64 / planets_total as f64);
        eprintln!("stats: mean planets {mean:.2}, giants {fg:.3}, neptunes {fn_:.3}, dwarfs {dwarfs}, belts {belts}, moons {moons}");
        assert!((5.0..12.0).contains(&mean), "mean planet count {mean}");
        assert!((0.07..0.17).contains(&fg), "gas giant share {fg}");
        assert!((0.2..0.4).contains(&fn_), "ice giant share {fn_}");
        assert!(dwarfs > 20 && belts > 20, "dwarfs {dwarfs}, belts {belts}");
        assert!(moons > 1000, "too few moons: {moons}");
    }

    #[test]
    fn exotic_stars() {
        // Every star but a black hole of 400+ solar masses gets SE's planets.
        for seed in 0..10u64 {
            assert!(generate_system(&star(500.0, StarKind::BlackHole, 1e-3, 0.01), seed).bodies.is_empty());
            let g = generate_system(&star(10.0, StarKind::BlackHole, 1e-4, 0.01), seed);
            assert!(g.bodies.iter().any(|b| b.tag == "Planet" || b.tag == "DwarfPlanet"));
        }
        // Neutron stars: SE's remnant disk (0.001-0.02 Earth masses) caps each type's mass draw,
        // which then falls between the cap and the type's floor (31.8 Earth masses for giants).
        for seed in 0..50u64 {
            let g = generate_system(&star(1.4, StarKind::NeutronStar, 1e-5, 1.0), seed);
            // (A neutron star with a companion also gets the companion's planets.)
            for b in g.bodies.iter().filter(|b| g.stars.is_empty() && (b.tag == "Planet" || b.tag == "DwarfPlanet")) {
                assert!(b.mass_me < 32.0, "pulsar planet of {} Earth masses", b.mass_me);
            }
        }
        // Giant: nothing inside 1.05 stellar radii.
        for seed in 0..100u64 {
            let s = star(1.2, StarKind::Normal, 40.0, 8.0);
            let g = generate_system(&s, seed);
            for b in g.bodies.iter().filter(|b| b.parent == s.name) {
                assert!(b.a_au > 1.05 * 40.0 * 0.00465, "planet inside the giant: {} AU", b.a_au);
            }
        }
        // M dwarf: the sphere of influence (40 AU per solar mass) keeps the system small.
        for seed in 0..100u64 {
            let s = StarParams { lum_bol: 5e-4, ..star(0.09, StarKind::Normal, 0.12, 5.0) };
            let g = generate_system(&s, seed);
            // (A wide binary's circumbinary planets lie further out.)
            for b in g.bodies.iter().filter(|b| g.stars.is_empty() && b.tag == "Planet") {
                assert!(b.a_au < 15.0 && b.t_eq_k < 3300.0, "{} a {} T {}", b.name, b.a_au, b.t_eq_k);
            }
        }
    }

    #[test]
    fn sample_has_expected_blocks() {
        // Over many seeds each kind of block appears at least once and parses.
        let (mut atm, mut oc, mut cl, mut life, mut ring, mut aur) = (0, 0, 0, 0, 0, 0);
        let s = sun_like("RS 1-2-3-4");
        for seed in 0..120u64 {
            let sc = generate_system_sc(&s, seed);
            let doc = crate::sc::parse(&sc);
            for it in &doc.items {
                let Some(b) = it.block() else { continue };
                atm += b.has("Atmosphere") as usize;
                oc += b.has("Ocean") as usize;
                cl += b.has("Clouds") as usize;
                life += b.has("Life") as usize;
                ring += b.has("Rings") as usize;
                aur += b.has("Aurora") as usize;
                if let Some(a) = b.sub("Atmosphere") {
                    let m = a.str("Model").unwrap();
                    assert!(
                        [
                            "Biogenic", "Chlorine", "Earth", "Ethereal", "Jupiter", "Mars", "Neptune", "Pluto", "Sun",
                            "Thick", "Thin", "Titan", "Venus"
                        ]
                        .contains(&m),
                        "unknown atmosphere model {m}"
                    );
                }
                assert!(b.str("DiffMap").is_none());
            }
        }
        // Planets and moons leave their atmosphere, ocean and clouds to SE's completion.
        assert!(atm == 0 && oc == 0 && cl == 0 && life > 0 && ring > 0 && aur > 5, "{atm} {oc} {cl} {life} {ring} {aur}");
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        let kinds = [
            StarKind::Normal,
            StarKind::WolfRayet,
            StarKind::WhiteDwarf,
            StarKind::BrownDwarf,
            StarKind::NeutronStar,
            StarKind::BlackHole,
            StarKind::Carbon,
            StarKind::Subdwarf,
        ];
        let vals = [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-12, 1e9];
        for (i, &v) in vals.iter().enumerate() {
            for &k in &kinds {
                let s = StarParams { name: format!("X{i}"), mass_sol: v, lum_bol: v, teff: v, radius_sol: v, age_gyr: v, feh: v, kind: k, class: None };
                for seed in 0..5 {
                    let g = generate_system(&s, seed);
                    let bad = g.sc.lines().filter(|l| !l.trim_start().starts_with("//")).find(|l| l.contains("NaN") || l.contains("inf"));
                    assert!(bad.is_none(), "{bad:?}");
                }
            }
        }
    }

    #[test]
    fn engine_seeds_vary() {
        // The engine seeds each star by its name's hash (`proc_surface::name_seed`).
        let mut seeds = std::collections::HashSet::new();
        let mut multi = 0;
        for i in 0..200 {
            let s = sun_like(&format!("RS 16-37200-41386-32741-{i}"));
            let seed = crate::proc_surface::name_seed(&s.name);
            seeds.insert(system_seed(&s, seed));
            multi += is_multiple(&s, seed) as usize;
        }
        assert_eq!(seeds.len(), 200);
        assert!((70..130).contains(&multi), "{multi} of 200 multiple");
    }

    #[test]
    fn multiple_stars() {
        // SE: binaries 50% (ProbBinaryStar), each component splitting again with 11%, theirs 22%.
        let (mut multi, mut more, mut circumbinary) = (0usize, 0usize, 0usize);
        for seed in 0..400u64 {
            let s = sun_like(&format!("RS 5-5-5-{seed}"));
            let g = generate_system(&s, seed);
            assert_eq!(is_multiple(&s, seed), !g.stars.is_empty());
            if g.stars.is_empty() {
                continue;
            }
            multi += 1;
            more += (g.stars.len() >= 3) as usize;
            assert!(g.stars.len() <= 8);
            let cat = load_generated(&s, &g);
            let mut total = 0.0;
            for (n, _) in &g.stars {
                let Some(ObjectRef::Star(i)) = cat.find(n) else { panic!("{n} missing") };
                let st = &cat.stars[i as usize];
                assert!(!st.barycenter && st.mass_kg > 0.0, "{n}");
                let o = st.orbit.as_ref().unwrap_or_else(|| panic!("{n} has no orbit"));
                assert!(o.period > 0.0 && o.semi_major_axis > 0.0 && o.eccentricity < 0.9, "{n}");
                total += st.mass_kg / crate::units::SOLAR_MASS_KG;
            }
            // SE fixes a barycentre's mass when it splits; a later split of a component re-derives
            // the halves' masses from their light, or keeps the star whole beside a remnant
            // companion of up to its own mass, so the stars' total can grow up to twofold.
            let root = g.root_mass_sol.unwrap();
            assert!(total > 0.5 * root && total < 2.5 * root, "seed {seed}: stars {total} vs root {root}");
            if g.stars.len() == 2 {
                assert!((total - root).abs() < 1e-4 * root, "seed {seed}: stars {total} vs root {root}");
            }
            circumbinary += g.bodies.iter().any(|b| b.parent == s.name) as usize;
        }
        eprintln!("multiples {multi}/400, 3+ stars {more}, with circumbinary planets {circumbinary}");
        assert!((150..=250).contains(&multi), "multiples {multi}");
        let f = more as f64 / multi as f64;
        assert!((0.08..0.35).contains(&f), "3+ star share {f}");
        assert!(circumbinary > multi / 2, "circumbinary {circumbinary}");
    }

    #[test]
    fn spin_and_period_consistency() {
        let s = sun_like("RS 7-7-7-7");
        let mut checked = 0;
        for seed in 0..60u64 {
            let g = generate_system(&s, seed);
            let cat = load_generated(&s, &g);
            for sum in g.bodies.iter().filter(|b| b.tidally_locked) {
                let ObjectRef::Body(i) = cat.find(&sum.name).unwrap() else { panic!() };
                let b = &cat.bodies[i as usize];
                let t_orb = b.orbit.as_ref().unwrap().period;
                let t_rot = b.rotation.period_seconds().unwrap();
                assert!(((t_rot - t_orb) / t_orb).abs() < 1e-3, "{}: rot {t_rot} orb {t_orb}", sum.name);
                checked += 1;
            }
        }
        assert!(checked > 20);
    }

    #[test]
    fn single_stars_get_se_comets() {
        let s = sun_like("RS 8514-491-6-169551-68");
        let mut checked = 0;
        for seed in 0..30u64 {
            if is_multiple(&s, seed) {
                continue;
            }
            let g = generate_system(&s, seed);
            let cat = load_generated(&s, &g);
            let comets: Vec<_> = cat.bodies.iter().filter(|b| b.tag == BodyTag::Comet).collect();
            assert!((100..200).contains(&comets.len()), "seed {seed}: {}", comets.len());
            assert_eq!(comets[0].names[0], format!("{}C1", s.name));
            for b in &comets {
                let o = b.orbit.as_ref().unwrap();
                assert!(o.eccentricity > 0.5 && o.eccentricity < 1.0);
                let tail = CometTailGiven::from_body_block(&b.block);
                assert!(tail.max_length_au.unwrap() > 0.0 && tail.particles.unwrap() >= 1000);
            }
            checked += 1;
        }
        assert!(checked > 5);
        // None around a brown dwarf.
        let bd = StarParams { mass_sol: 0.05, lum_bol: 1e-5, teff: 1200.0, radius_sol: 0.1, kind: StarKind::BrownDwarf, ..sun_like("BD") };
        let g = generate_planets(&bd, 3);
        assert!(!g.bodies.iter().any(|b| b.tag == "Comet"));
    }
}
