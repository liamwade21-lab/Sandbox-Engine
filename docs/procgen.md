# Procedural planetary systems (`se_core::procgen`)

Code: `crates/se_core/src/procgen/mod.rs`. The generator turns one star plus a seed into SpaceEngine
catalog script text (`.sc`, the dialect SE's own "export planet script" writes). The engine loads it
with the existing `Catalog::load_sc(text, source)` followed by `Catalog::finalize()` (a small public
wrapper around the private `resolve()`), so procedural bodies get orbits, rotation, `Atmosphere` /
`Clouds` / `Ocean` / `Life` / `Rings` / `Aurora` blocks and render through the normal path. Procedural
bodies reference **no texture maps** (no `DiffMap`, `BumpMap`, ring `Texture`).

## API

```rust
pub struct StarParams { pub name: String, pub mass_sol: f64, pub lum_bol: f64, pub teff: f64,
                        pub radius_sol: f64, pub age_gyr: f64, pub feh: f64, pub kind: StarKind,
                        pub class: Option<SpectralClass> }
pub fn generate_system_sc(star: &StarParams, seed: u64) -> String;
pub fn generate_system(star: &StarParams, seed: u64) -> GeneratedSystem; // { sc, bodies, stars, root_mass_sol }
pub fn generate_planets(star: &StarParams, seed: u64) -> GeneratedSystem; // a catalog star: never split
pub fn is_multiple(star: &StarParams, seed: u64) -> bool;               // SE makes it a barycentre
pub fn procedural_star_name(galaxy_code: &str, cell: [i64; 3], level: u8, index: u32) -> String;
pub fn cell_node_id(cell: [i64; 3]) -> u64;
pub fn planet_name(star: &str, n: usize) -> String;
pub fn temperature_class(t_k: f64) / pressure_class(p_atm) / volatile_class(ocean, ice) / size_prefix(bulk, mass_me);
pub struct Rng; // SplitMix64: new, from_name, fork, f64, range, log_range, normal, chance, below
```

Usage: `cat.load_sc(&format!("Star \"{n}\" {{ ... MassSol m }}"), ..)` is the engine's job (the star
comes from its star octree); then `cat.load_sc(&generate_system_sc(&star, seed), "procgen");
cat.finalize();`. The star record must carry `MassSol` equal to `StarParams::mass_sol`, because the
engine derives planet periods from the parent mass (the generator omits `Period`, and computes tidal-lock
spin periods with the same constants as `units.rs`). Determinism: output is a pure function of
`(star, seed)`; each planet, moon set and belt draws from its own forked RNG stream, so editing one part
of the rules does not reshuffle the others. The seed is mixed with an FNV hash of the star name.

## Output format

Per body, in SE key order: `ParentBody`, `Class`, `Mass` (Earth masses), `Radius` (km), `Oblateness`,
`Color`, `AlbedoBond`/`AlbedoGeom`, simple rotation keys (`RotationPeriod` h, `Obliquity`,
`EqAscendNode`, `RotationOffset`), `Surface { SurfStyle, Randomize, BumpHeight, [BumpOffset,
SpecBrightWater, colorSea, colorShelf] }` (procedural seeds only), `Interior.Composition`, `Ocean`, `Clouds`,
`Atmosphere`, `Life`, `Aurora`, `Rings`, `NoRings`/`NoAtmosphere`/`NoOcean`/`NoClouds`/`NoAurora`
flags, `Orbit`. Orbits are `SemiMajorAxis` (AU), `Eccentricity`, `Inclination`, `AscendingNode`,
`ArgOfPericenter`, `MeanAnomaly` (deg) and no `Period`/`Epoch` (engine: Kepler's third law, J2000).
Tags: `Planet` (>= 0.01 Earth masses), `DwarfPlanet`, `Moon` (radius >= 200 km), `DwarfMoon`, `Asteroid`.
Comments above each body give the descriptive class and temperatures.

`RefPlane`: planets/dwarf planets/asteroids use `"Ecliptic"` with a random system orientation
(`Inclination`/`AscendingNode` drawn isotropically for the system, then per-body tilts), moons use
`"Equator"` (the engine's `ParentEquator`: inclinations are relative to the planet's equator).
Real `ExoPlanets*.sc` omit `RefPlane` for planets (and `ExoPlanetsSuns-bin.sc` uses `"ExtraSolar"` for
binary stars); the engine currently maps anything but `Equator`/`Extrinsic`/`EarthEquator` to the
ecliptic. Writing `"Ecliptic"` explicitly keeps orbit frame and spin frame (the simple rotation model
is ecliptic-based) consistent, which `"Extrasolar"` would break once the engine gives it a different
meaning.

## Naming

Verified against `SpaceEngine/save/places-def.cfg`, `data/locale/*-db.cfg` and exe strings:
* Stars: `RS <galaxy-code>-<level>-<node>-<index>`, e.g. `RS 8514-491-6-169551-68` (galaxy code
  `8514-491`, octree level 6, node 169551, star 68) and `RS 0-8-11084762-901-49-7-384587-1004`
  (galaxy `0-8-11084762-901-49`, level 7, node 384587, star 1004). The exe contains `RG %i-%i-%I64d-%i`
  (galaxy), `RN`/`RC` (nebula/cluster) and a bare `%i-%i-%I64d-%i` format (the galaxy, cluster and nebula
  names are decoded in `docs/formats/procedural_deepsky.md`). SE's node numbering itself
  was not decoded: `procedural_star_name` packs the `[i64;3]` octree cell into the node id by 3-D Morton
  interleaving (21 bits/axis, biased by 2^20). Same galaxy code + cell + level + index => same name.
* Planets: `<star> <n>` (n = 1.. outward by semi-major axis, dwarf planets share the numbering);
  after a binary component letter the number is attached: `<system> A4`, `<system> B2`, `<system> AB3`
  (`planet_name` treats a trailing 1-3 capital letters as a component). Circumbinary planets of the
  root barycentre are `<system> <n>`.
* Moons: `<planet>.<n>` outward (round moons, tag `Moon`) and `<planet>.S<n>` (small/irregular
  `DwarfMoon`s). SE's data shows `4.1`, `5.1`, `B2.1` and letter forms (`3a`, `A6b`) and `2.S24`; the
  choice "numbers for round moons, `.S` for small ones" is an interpretation. Belt asteroids: `<star> S<n>`
  (designed).

## Which stars get planets

Every star, as in SE (`seSolarSystem::CreateProcedural`, FUN_1404f7c40), except black holes of 400 or
more solar masses, which SE hands to another generator (FUN_1404fb950, not ported). SE also hands
planemos (luminosity code 12) to another generator. The star's luminosity class and spectral type come
from `StarParams::class` (`climate::star_class_bits`), or are inferred from its kind, radius and
temperature.

## Multiple stars (SE's, `procgen/se_multiple.rs`)

A procedural star may become SE's multiple system (CreateProcedural FUN_1404f7c40 and the binary branch
of FUN_1404f9210). The engine then adds its record as a barycentre (`is_multiple`). The generator writes
the component `Star`s (and nested `Barycenter`s) with their orbits (`Period` given), and sets the root's
mass to the stars' total (`root_mass_sol`). Catalog stars are never split (`generate_planets`), as in SE.

* **Hierarchy**: A/B with ProbBinaryStar 0.5 (0.95 for luminosity code 10). A and B each split with
  ProbTripleStar 0.11, and each of their halves with ProbQuadrupleStar 0.22. That gives up to eight
  stars, named `<system> A`, `B`, `AA`, `AB`, `BA`, `BB`, `AAA` ... `BBB` (SE's suffix strings).
  A sub-split is capped at 0.3 of the component's sphere of influence. Stars of at most 0.045 Msun or
  9.33e-6 Lsun never split.
* **Companions** (2.25% remnants): a black hole of `tn(8, 3, 3, 100)` Msun (ProbCompanionBH 0.0075), a
  neutron star of `tn(2.22, 0.39, 1.44, 3)` (NS 0.005), or a white dwarf of `tn(1, 0.5, 0.56, 1.44)`
  (WD 0.01). The star keeps its light; a remnant heavier than the star takes the primary's place. A
  primary older than its lifetime becomes the main-sequence star whose lifetime is the age:
  `M = (age/10 Gyr)^-0.2667`.
  Otherwise, with ProbBrownDwarf (0.025, x0.2 after each one), a brown dwarf of
  `tn(0.03, 0.02, 0.013, 0.08)` Msun. Otherwise the light splits: A gets `L(q M)`, at least half,
  with `q = tn(0.75, 0.3, 0.01, 0.99)` (`tn(0.65, 0.07, 0.5, 0.8)` for code 10), and B, a
  main-sequence star, gets the rest. The heavier star is A. `L(M)` is SE's mass-luminosity law
  (FUN_1401dd880):

  | mass | `L` |
  |---|---|
  | > 55 | `32000 M` |
  | > 2 | `1.4 M^3.5 + 0.00303 (55 - M)` |
  | > 0.85 | `M^4` |
  | > 0.2 | `M^(232.4 M^3 - 141.7 M^4 - 129.1 M^2 + 33.29 M + 0.215)` |
  | > 0.1 | `0.23 M^2.3` |
  | otherwise | `1237.6 M^6.03` |

* **Orbit**: `log10(a/AU) = tn(log10(cap)/1.84, 2.5, -2, log10(cap))` (-3 for code 10), at least
  2.5 (R_A + R_B). The cap is 1000 AU for a field star (assumed, as for the sphere of influence).
  `e = tn(0.5, 0.5, 0, 0.89)`, zero below a periapsis of 2.5 (R_A + R_B), scaled by
  `(log10 a + 1)/0.7` up to 1. The barycentre's plane is random; the components orbit within
  +-2.5 degrees of it, B opposite A.
* **Planets**: every component star gets `rand % 10 + 5` slots. Its sphere of influence is the
  periapsis separation / (`sqrt(M_other/M/0.05) + 1`). The root barycentre gets `rand % 10 + 5`
  circumbinary slots out to `max(10 apoapsis, tn(40, 8, 10, 1000) M)`. Its planets start beyond the
  larger of its components' inner edges and `3 x` their apoapsis separation `+ 1.05 (R_A + R_B)`
  (FUN_14050a890). A sub-barycentre keeps the slots it drew as a star.
* **Not decoded**: SE's class tables (FUN_1401d6a50 and the table at 0x140807d40, built at start-up),
  FUN_1405045a0's star physics, and the remnants' properties (FUN_1401d5fe0).
  - A component that keeps the original class keeps its temperature, its radius scaled by
    `sqrt(L_A/L)`, and a mass from its light (dwarfs) or `q M`.
  - A new main-sequence component takes the port's temperature-luminosity relation and SE's mass law.
  - White dwarfs get the port's temperature draw (5000-30000 K), neutron stars 600000 K and 12 km,
    and black holes a 2.953 km per Msun radius.
  - The split decisions draw from per-star streams of the port's.
  - SE fixes a barycentre's mass when it splits, so after a sub-split the stars' total can differ
    from the root's (up to twice it beside a remnant companion), as in SE.

## Planet architecture (SE's, `procgen/se_layout.rs`)

Decoded from FUN_1404f9210 (single star), FUN_140500700 (layout), FUN_14050a9f0 (inner limit),
FUN_140501f60 / FUN_140504e10 (planet type and mass), FUN_1401aa500 / FUN_1404df8a0 / FUN_1401abd00
(composition and radius) and FUN_1404fd2a0 (asteroid belts). The probabilities are main-def.cfg's
`SolarSystemGenerator` values, matched to their exe globals through the config reader (FUN_14027a6a0).

* **Slots and reach**: `rand % 10 + 5` planet slots. The sphere of influence is
  `tn(40, 8, 10, 1000) M/Msun` AU, capped by SE at 10-1000 AU from the star's surroundings; a field
  star gets the 1000.
* **Inner limit**: the largest of 1.05 stellar radii, the distance where `T_eq` falls to 800 K, and
  `4 x 15545 km x cbrt(2 M/Me)`. The `T_eq` limit is 3200 K when hot planets are allowed: giants,
  luminosity code 9, or `U < ProbHotGasGiant + ProbHotIceGiant + ProbHotTerra` (6.2%). Remnants use
  5800 K, codes 12+ use 1e8 K.
* **Spacing**: the first distance is `U(1, 1.1)` x the inner limit, then each slot multiplies it by
  `U(1.2, 2.7)` with `e = |N(0, 0.06)|`. 7% of slots instead step by `U(1.7, 2.2)` with
  `e = |N(0.2, 0.1)|`, and add 70-140 degrees of inclination where the star's surroundings are dense
  (generator +0xa4 > 0.5; never for the port's field stars). After an inclination above 45 degrees, the
  next step is a further `x U(2.2, 2.5)`. Placement stops past the sphere of influence. A planet with
  `T_eq` above 3200 K is dropped without using its slot.
* **SE's overlap guard**: SE compares the new semi-major axis (AU) with 1.01 x the previous apoapsis read
  back in parsecs, so it practically never acts and orbits may cross. Reproduced as SE has it.
* **Orientation**: inclination `N(0, 0.0005 sqrt(P_days / (2 pi a_pc)))` + `U(-1.5, 1.5)` degrees.
  Node, argument of periapsis and mean anomaly are uniform. The port tilts these onto a random system
  plane (`RefPlane "Ecliptic"`).
* **Type and mass** (planet stream `(0xcdb6e3e3 - n 0x32491c1d) ^ seed`, `(n+1) 0x18f4b843 ^ seed`):
  `log10(M/M_J)` is drawn by type, with each upper limit capped at `log10` of the disk mass (1.2% of the
  star's; `min(1e-4 M, 5 Me)` for brown dwarfs; `tn(0.01, 0.005, 0.001, 0.02)` Me for remnant codes
  11/12-13):

  | type | probability | `log10(M/M_J)` |
  |---|---|---|
  | gas giant | ProbGasGiant 0.12 | `tn(0.5, 0.5, -1, 1.12)` |
  | ice giant | ProbIceGiant 0.25 | `tn(-1, 0.5, -2.5, -0.5)` |
  | terra | otherwise | `tn(-2.66, 1, -6.2, -1.2)` |

  ProbColdTerra (10%) makes a rocky body form as if at 600 K (dry), wherever it lies.
* **Composition**: gas giants get a Jupiter's, ice giants a Neptune's. A terra of at least
  `tn(10, 1.5, 6, 20)` Me gets a Neptune's, and one of at least `tn(60, 10, 30, 100)` Me a Jupiter's.
  Otherwise it is rocky (`climate::rocky_composition` with the planet's C/O, metallicity, `T_eq` and
  mass). The class follows: Jupiter, Neptune, or Ferria/Terra/Carbonia/Aquaria by composition. The
  generator writes it to `Interior.Composition` (Hydrogen, Helium, Ices, Silicates, Carbides, Metals),
  and SE's completion keeps a given composition.
* **Radius**: giants follow SE's fit in Jupiter masses and `T_eq`: inflated when hot, /1.8 below 60 Me,
  at most 1.5 R_J, so small hot ice giants are puffy. Solid bodies follow
  `R = r1 10^(log10(M/6.41)/3 - 0.25 - 0.14565 (M/6.41)^0.27) Re`, with r1 interpolated in SE's tables
  by the ice share (5.742 pure ice to 4.237), else the carbon share (4.279 to 3.125), else the silicate
  share (4.237 to 3.125 for iron).
* **Dwarf planets and belts**: a body of at most 2000 km is a `DwarfPlanet` and carries
  `100 + rand % (int(min(a^3, 10)) 1000 / 3)` asteroids, at most 1000 per system. They lie between the
  neighbours' orbits: inside, an apoapsis plus 7 Hill radii; outside, a periapsis less 2.8 Hill radii;
  where there is no neighbour, the dwarf planet's own periapsis or apoapsis. `e = tn(0.1, 0.05, 0, 0.3)`,
  `a ~ N(a_dp, width/3)` clamped, inclination `N(0, 2e-7 P/(2 pi a_pc))` + `U(-2.5, 2.5)` degrees. A
  dwarf planet also gets `N(0, 8)` degrees more inclination and `N(0, 2.5)` more node, times
  `min(sqrt(a), 1)`. The asteroids' sizes are the port's (3-250 km): SE's FUN_140502710 is not decoded.
* **Moon counts**: by gas share (H + He) of the composition: above 0.25, `N(60, 20)` clamped to 5-100;
  between 0 and 0.25, `N(20, 8)` clamped to 0-40; solid planets `N(10, 8)` clamped to 0-20; dwarf planets
  `N(3, 2)` clamped to 0-10. A third of that around brown dwarfs, plus 1 when any. A ProbRingsDebris (3%)
  ring of `N(300, 100) sqrt(M/M_J)` bodies (at most 1000) also counts in SE; the port does not generate
  those bodies. SE's moon generator (FUN_1404fdc90, about 1350 lines) is not ported, so the port places
  SE's number of moons with its own rules (below).
* **Not generated**: SE's 100-199 comets per star.
* **Assumed**: the star's C/O (0.55) and metallicity factor (`10^[Fe/H]`), which SE's octree draws. The
  star's seed is the port's (the port's stars are not SE's octree stars), so a system follows SE's rules
  without matching SE's system for the same star. SE's draws inside FUN_1405045a0 / FUN_140504070 /
  FUN_1404ca7e0 are not replicated.

## Bodies

* **Radius** (km): solid, M < 0.01 Me: from density (`1/rho = w/1000 + (1-w)/rho_rock`,
  `rho_rock = 3300 + 4000 (cmf-0.33)`); M >= 0.01 Me: `6378.14 (1 - 0.27 (cmf-0.33)) (1 + 0.75 w) M^0.30`
  (0.27 above 1 Me), calibrated on Moon/Mars/Mercury/Venus/Ganymede/Titan/Europa and Zeng-style super-Earths.
  Envelope planets: `0.867 M^0.55` (Chen & Kipping) x U(0.9, 1.1), never below 1.15 x the rocky radius.
  Gas giants (M >= 60 Me): log-interpolation Neptune curve -> Saturn (95.2 Me, 9.45 Re) -> Jupiter
  (317.8 Me, 11.2 Re) -> `(M/318)^-0.04`; hot Jupiters inflated up to +25 % above 1000 K.
* **Composition**: `cmf` (iron fraction of the rock part) `N(0.32 + 0.05 [Fe/H], 0.06)`; Ferria if
  cmf >= 0.5 (5 %, 25 % very close in); Carbonia 3 % of dry bodies; water/ice mass fraction `w`: 0.2-0.5
  beyond 1.1 x snow line, 0.05-0.3 for 10 % of mid-zone planets, ~0 inside. `Class`: Aquaria if
  w >= 0.15, Carbonia, Ferria, else Terra; Jupiter (>= 60 Me gas giant), Neptune (envelope), Chthonia
  (hot envelope planet stripped to its core: T_eq > 1500 K, < 20 Me, p 0.5), Asteroid.
* **Albedo / temperature**: `T_eq = 278.5 K L^(1/4) a^(-1/2) (1-A_B)^(1/4) (1-e^2)^(-1/8)`; surface
  temperature `T_s = T_eq + Greenhouse`, with `Greenhouse = coef sqrt(p)` (coef 52 CO2, 33 N2/H2O/O2, 10
  N2/CH4, <= 520 K) and an extra 0-70 K carbonate-silicate term on cold wet worlds. Calibration:
  Earth 33 K, Venus ~500 K, Mars ~4 K, Titan ~12 K.
* **Atmosphere retention**: empirical cosmic shoreline `S x XUV < (v_esc / 7 km/s)^4` x 10^(0.25 N(0,1))
  (`S` in Earth insolations, XUV multiplier 1..9 for M < 0.7 Msun). Reproduces Earth, Venus, Titan, Triton,
  Pluto (kept) and Moon, Mercury, Callisto, Ganymede (lost). Regime by `T_eq(A=0.3)`: >= 262 K hot
  (Venus-like CO2, p ~ 10^N(1.4+0.5 log M, 0.45) atm, 70 % bright clouds if p > 20; rare chlorine / steam
  variants), 208-262 K temperate (55 % wet for M >= 0.25 Me: N2/O2/CO2/H2O, p ~ 10^N(0, 0.3);
  otherwise dry CO2 or N2 desert), 150-208 K cold (thin Mars-like, thick CO2 greenhouse super-Earth, or
  ice), < 150 K frozen (Titan-like N2/CH4 for M >= 0.015 Me, Pluto/Triton-like thin N2/CH4/CO).
  Water worlds below 258 K flip to a snowball (A_B 0.5-0.6, ice cover, no `Ocean`); oceans that exceed
  the boiling point are removed.
* **Atmosphere block**: `Model` from the shipped set. Hot thick CO2: `Venus` (p > 20) / `Thick` /
  `Mars`; Earth-like wet: `Earth`; dry N2: `Thin` (< 0.3 atm) / `Earth`; cold CO2: `Mars` / `Thin` /
  `Thick`; Titan regime: `Titan`; Pluto regime: `Pluto`; exotic life: `Biogenic`; chlorine: `Chlorine`;
  gas giants `Jupiter`, ice giants `Neptune`. `Height` = 8-11 scale heights; `Opacity`, `Bright`,
  `SkyLight` are interpolated in log pressure through Pluto/Triton (1e-5 atm), Mars (6e-3) and Earth/Venus
  (>= 1) reference values from `SolarSys.sc`; `Density` from the ideal gas law; `Greenhouse` in K;
  `Composition` in percent. `Ethereal` and `Sun` are never emitted (reserved for exotic/airless cases).
  Giants copy the SE menu planet: `Pressure 1e6`, `Density 1000`, `Bright 10`, `Opacity 1`,
  `SkyLight 1`, plus `Hue`/`Saturation` by temperature class.
* **Giant colours/clouds**: Sudarsky-like by `T_eq(A=0.3)`: < 150 K cream (A_B 0.3-0.38), 150-250 K white
  (0.5-0.65), 250-800 K azure (0.08-0.16), 800-1400 K dark (0.03-0.08), > 1400 K reddish (0.2-0.4); ice
  giants cyan/blue. `Clouds` carry the SE stripe/cyclone parameters (`mainFreq`, `stripeZones`, ...).
* **Ocean / terrain**: `Surface.BumpHeight = 19.5 km (9.81/g)^0.8 x U(0.7, 1.4)` (1.5-60);
  `Ocean.Height = BumpOffset = BumpHeight (0.15 + 0.65 f)` for cover `f`, or 1.2-3 x BumpHeight for global
  oceans; salinity 1.5-6 % NaCl. (Earth: 19.475 / 10.896 km.)
* **Life**: needs liquid ocean, 268 K <= T_s <= 332 K, 0.3-6 atm, age > 0.5 Gyr: 35 % (20 % around
  active M dwarfs); multicellular if age > 2 Gyr (60 %), O2 8-32 % if age > 2.4 Gyr, `Biome "Marine"` or
  `"Marine/Terrestrial"`, 6 % `Exotic` class.
* **Rings**: gas giants 10 % (40 % if > 60 Me and T_eq < 400 K), ice giants 25 %; inner 1.15-1.5 R, outer
  capped at 0.95 x Roche limit (`2.456 R (rho_p/0.9)^(1/3)`); 25 % of the cold ones are bright
  Saturn-like, others faint. Moons stay outside 1.2 x the outer ring radius. Other bodies get `NoRings`.
* **Aurora**: giants 85 %, atmosphered rocky planets (> 0.05 atm, > 0.3 Me) 40 %.
* **Rotation**: unlocked: rocky 6-60 h (12 % slow 60-1500 h), gas giants 8.5-18 h, envelope planets 13-21 h,
  small bodies 2.5-40 h; 2 % retrograde; hot thick-CO2 worlds 50 % slow (1500-6000 h, mostly retrograde).
  Obliquity relative to the orbit: 70 % Rayleigh(12 deg), 20 % 20-60, 10 % 60-140 (moons of a planet:
  Rayleigh(2 deg)); converted to the engine's ecliptic `Obliquity`/`EqAscendNode` by tilting the orbit normal
  (`normal = (sin i sin Omega, -sin i cos Omega, cos i)`). Tidal locking uses the Gladman et al.
  timescale with `k2 = 1.5/(1 + 19 mu/(2 rho g R))`, Q = 100 (rocky/icy) or 1e5 (giants) and an initial 10 h
  spin; locked => `RotationPeriod` = orbital period, pole along the orbit normal, prime meridian toward the
  parent at J2000 (approximate for moons because the engine's parent-equator node reference is not
  replicated); eccentric (> 0.2) planets get a 3:2 resonance 70 % of the time. Oblateness from
  `0.7 q` (giants) / `1.0 q` (rocky), `q = w^2 R^3 / GM`, capped at 0.35; 0 when locked.
* **Moons** (SE's count from the layout; giants: regular moons up to the port's own draw, the rest
  irregular; solid bodies: all small moons; the bounds below otherwise): regular moons for giants (0-7, total mass `1e-3.7 M_p`, weights `10^(-0.35 i)`, spacing
  x1.4-2.1 outward from 1.3 x Roche), 0-6 for envelope planets, 0-3 small ones for rocky planets > 0.003 Me,
  25 % chance of a Moon-like large moon (mass ratio 10^-3 to 10^-1.7) for M >= 0.1 Me, up to 3 small moons of
  dwarf planets (one with mass ratio 0.003-0.15), plus 0-6 irregular captured satellites (eccentric,
  inclined, 50 % retrograde) beyond the regular ones. Hill rule: regular moons need apocenter < 0.24 R_Hill,
  everything < 0.45 R_Hill (test: < 0.5). Planets inside 0.1 AU keep each moon with probability 0.15.
  Moon composition: icy beyond the snow line (inner two rock-rich half the time), rocky inside.

## Classification words (`BodySummary::class_desc`)

SE builds descriptions as `<temperature> <atmosphere> <volatiles> <size><bulk> [with life]`
(`PlanetClassOrder*` in `eng-gui.cfg`). **Verified** word lists (eng/cat-gui.cfg and exe strings):
temperature torrid, hot, warm, temperate, cool, cold, frigid; atmosphere airless, infrabaric,
hypobaric, mesobaric, hyperbaric, ultrabaric; volatiles arid, lacustrine, marine, oceanic, superoceanic,
glacial, superglacial; size micro, mini, sub, (none), super, mega + bulk terra/ferria/carbonia/oceania/aquaria/
chthonia/titan/neptune/jupiter (+ ice/gas/helium giants, `earth`). The `Class` key value set in the exe
is Terra, Titan, Aquaria, Jupiter, Neptune, Ferria, Carbonia, Chthonia (+ Asteroid, BrownDwarf); the
generator uses Terra, Ferria, Carbonia, Aquaria, Neptune, Jupiter, Chthonia, Asteroid (`Selena`, `Glacia`
and `Lava` do not exist in this SE build). **Designed** thresholds: temperature 130/240/273/310/350/600 K,
pressure 1e-7/1e-3/0.1/10/500 atm, ocean cover 0.1/0.75/0.97 and ice cover 0.3/0.95, terra mass limits
1e-3/0.05/0.5/2/10 Me, Neptune 6/12/30/60 Me, Jupiter 30/150/1000/2500 Me.

## Verified vs designed

Verified (read from this repo / SE files): the `.sc` dialect and every key used above (docs/formats,
`catalog.rs`, `body_params.rs`, `rings.rs`, `rotation.rs`, `orbit.rs`); Atmosphere model names (shipped
`.atm` files); procedural star/planet/moon name patterns (`places-def.cfg`, `*-db.cfg`); the class word
lists and class value set. **Decoded from SpaceEngine.exe**: the whole planet architecture above (slots,
sphere of influence, inner limit, spacing, eccentricities, inclinations, types, masses, composition,
radius, dwarf planets, asteroid belts, moon counts). Planets and moons also get SE's completion chain
(composition, atmosphere, ocean, clouds, temperature) when the engine loads them.

Designed here: moon orbits and sizes, asteroid sizes, the port-side physics the emitter still writes
(rotation, tidal locking, rings, aurora), the node-id packing of `procedural_star_name` and the `.S`/`.N`
moon-name interpretation.

## Tests (`cargo test -p se_core procgen`)

Determinism; RNG fork independence; name formats; generated text loads through `Catalog::load_sc` +
`finalize` for 40 seeds; SE's architecture over 200 Sun-like seeds (1-14 planets, mean 5-12, steps of at
least x1.19 outward, gas giants 7-17% and Neptunes 20-40% of planets, dwarf planets exactly the bodies of
at most 2000 km, belts of 100-1000 asteroids, Hill-stable moons outside their planets); exotic stars
(400+ Msun black holes have nothing, stellar black holes get planets, single pulsars' planets under 32 Me,
nothing inside a giant, single M-dwarf systems within 15 AU); multiple stars over 400 seeds (about half
multiple, 8-35% of them with 3+ stars, components loading with orbits, most with circumbinary planets); only shipped atmosphere models and no `DiffMap`; degenerate
input (NaN, 0, inf) does not panic; locked bodies' `RotationPeriod` equals the engine-resolved orbital
period. `se_layout::tests::print_sample` (ignored) prints sample systems.

## Terrain (`proc_surface::ProcSurface::height_m_lod`)

SE generates its terrain in encrypted shaders, so the port's height function is designed and was compared
with SE side by side on Proxima b (cold arid terra) on 2026-10-27. SE shows terraced plateaus with cliff
risers, long sinuous scarps, big eroded craters, dune fields at about 1.5 km spacing and smooth plains;
the port had broad noise only. `landforms` now adds plateaus (3 levels), scarps, canyons on dry worlds and
dunes on dry worlds with air, each band-limited to the patch's vertex spacing; dry thin-atmosphere worlds
keep craters (strength 0.3). The surface shader fades a sample texture's contrast in as its repeat grows
past 48 px on screen (no visible grid from orbit), adds albedo patches a few hundred km across, and planet
light keeps half of its star's colour saturation (a red dwarf's planet is tan, as in SE). Still different:
Since 2026-10-27 the per-planet parameters are SE's own: `se_core::surface_gen` ports the exe's
generator (docs/formats/surface_params.md), with SE's seeds for catalog bodies, so every body gets
the `SurfStyle`, classes, biome preset and landform/climate values SE gives it. Bodies without a
bump map then use `ProcSurface::se_height`: continents at `mainFreq`, maria, one landform per region
chosen by SE's cumulative shares (dunes, hills, rounded hills, canyons, mountain ranges with
`montesSpiky`), terraces on a `terraceProb` share, cracks, the equatorial ridge, and
`craterOctaves` crater levels, the largest about R / `craterFreq` across, in fractions of `BumpHeight` above
`seaLevel`. Craters sit one per cell of a 3-D lattice per level, centred on the sphere; only those whose
centre on the sphere stays in its own cell are kept, as a query only searches the neighbouring cells (a
crater pushed further was cut off along a cell face: a straight cliff hundreds of metres high across the
ground, `noise::tests::craters_are_continuous`). The shader places biomes from SE's climate values (`se_biome_weights`): a climate
index equator -> tropic band -> pole rising with height, SE's steppe/grass/forest ranges,
`snowLevel`, `icecapLatitude` and `beachWidth`. How SE's encrypted shader turns these values into
relief and colour is not known, so that composition is designed.

Level of detail (`terrain.rs`): a 33x33 patch splits within 2.2 edge lengths (times the quality's
`LandLOD` factor). Each vertex also stores where its parent patch had the surface (the parent's
own vertices with their coarser detail, and the midpoints of its straight edges and diagonals);
the vertex shader moves from there to the patch's own shape between twice and 1.4 times the split
distance, so a split shows no jump. Shading uses a per-patch normal map of 65x65 texels (body-frame
normals, detail down to one vertex spacing), twice the mesh's resolution.

Catalog planets and moons whose entries lack an `Atmosphere` get SE's generated one (docs/formats/climate.md,
`se_core::climate`): bulk composition, tides and heat, a gas inventory per species with loss and
escape, and a pressure/temperature/phase solve giving pressure, gases, greenhouse, albedo, clouds and
oceans. The results feed the classes, presets, terrain (ocean, `BumpHeight`) and a sky shell with
SE's choice of scattering model. Proxima b now gets a thin arid atmosphere and
`terra_arid_rocky_black.cfg` (was `terra_airless_io.cfg`); Proxima d gets water lakes.
`BumpHeight` without a catalog value is SE's default, min(20 / sqrt(g), 20, 0.05 R) km.

## Not included / ideas

Binary stars, comets, tidal-heating / Io-like volcanism, Trojans, resonant chains, rings of rocky
planets, planetary magnetic-field model beyond the aurora heuristic, `Climate` block, asteroid shape
(`Dimensions`) and thousands of belt objects.
