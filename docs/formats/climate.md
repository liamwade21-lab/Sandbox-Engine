# Planet completion: atmosphere, ocean and climate (SpaceEngine 0.991)

SE fills in what a catalog planet's or moon's entry leaves out. A body without an `Atmosphere`
block (and without `NoAtmosphere`) gets a generated atmosphere, and from it an ocean, clouds,
temperature, albedo and classes; giants get a hydrogen-helium envelope. Decoded from SpaceEngine.exe 0.991; ported in
`se_core::climate` (wired into `surface_gen::catalog_input`, the terrain and the sky shell).

Data tables are read from the exe at runtime (never copied into the repo):

| VA | Contents |
|---|---|
| 0x140674e30 | 28 gas species, 104 bytes each: name[32], then 18 floats (below) |
| 0x140676380 / 0x1406763a0 | XUV share of L_bol, subtype 5 and subtype 0 of O..M (8 floats each) |
| 0x14067a640 | atmosphere model names: Sun, Venus, Earth, Mars, Titan, Neptune, Pluto, Jupiter, Chlorine, Thick, Ethereal, Thin, Biogenic |

Species fields: 0 triple T, 1 triple P (atm), 2 critical T, 3 critical P, 4 boiling T at 1 bar,
5 latent heat (kJ/mol), 6 molar mass, 7 liquid density, 8 solid density (g/cm^3), 9-11 Antoine
A/B/C, 12/14 outgassed inventory (x1e6) for metal-poor / metal-rich bodies, 13/15 inventory a cold
body keeps, 16 photochemical loss rate, 17 unknown. Order: H2, He, Ne, Ar, Kr, Xe, O2, N2, NH3,
H2O, CO, CO2, SO2, Cl2, H2S, CH4, C2H2, C2H4, C2H6, C3H8, C8H18, SO, NaCl, SF6, CF4, C2F6, C3F8,
C4F10. Only the first 21 take part in the phase solve, losses and escape.

The composition array (record +0x1480) holds 28 gas, 28 liquid and 28 ice inventories, then gas,
liquid and ice sums, the total inventory (Earth atmosphere masses), mean liquid density, ocean
depth, ocean fraction, cloud cover, ice fraction, albedo and flags.

## Where the inputs come from

- **Star keys.** Star records hold `Lum`, `LumBol`, mass, radius, `Teff`, `FeH`, `CtoO` and `Age`
  (3-letter keys, so a strings dump with a 4-character minimum misses `FeH`/`Age`). A planet's
  composition parameter (record +0x1614) is the host's `CtoO` times U(0.8, 1.2) (one stream-A
  draw); unset without `CtoO`, which makes the mix `f = sat((CtoO - 0.7) * 10)` 0: metal-poor
  columns and no carbon. `FeH` scales the same way into +0x1610.
- **System age** (FUN_1404f62e0): the largest `Age` of the system's objects; without one,
  `U * min(12.87, lifetime of each star)` Gyr, `U` the next stream-A value after the system seed.
  SE builds that seed from bits of the star's 128-bit galactic position; the port draws `U` from
  the root star's name seed instead.
- **Stellar lifetime** (FUN_1401dd7e0): `10 * M^(-1.125 - 3.75 M)` for M <= 0.7 M_sun, else
  `max(10 * M^-3.75, 0.002)`; x1.1 for luminosity classes up to IV; 12.87 for remnants.
- **Flux** (FUN_14050abe0): sum of `LumBol / d_AU^2` over the system's stars, with every level
  of the body's hierarchy placed at periapsis (FUN_1404ea7d0) and the distance scaled by a mode
  using the eccentricity of the body's own orbit (its planet's, for a moon): x1 (mode 0, read by
  the completion) or x(1 + e^2/2) / (1 - e) (mode 2, read by the temperature model below). For a
  planet of a star that is periapsis and the mean distance a(1 + e^2/2); Earth, which orbits the
  Earth-Moon barycentre with the Moon's eccentricity, gets 0.92 (`climate::reference_distances_au`).
  XUV adds
  `xuv_fraction * LumBol / d^2`. `xuv_fraction` interpolates the two tables by subtype below 5
  and takes the next type's subtype-0 value from 5 on (an exe quirk: M5-M9 dwarfs get 0).
  `T_eq = max(20, 278 * ((1 - A) * flux)^0.25)`. `A` is the Bond albedo as SE's loader sets it
  (FUN_140468860): `AlbedoBond`, else `Albedo` (which sets both albedos), else `AlbedoGeom`; 0.3
  with none. A given albedo stays fixed through the solve below (no cloud or ice albedo);
  `T_exo = max(T_eq, 1500 * (xuv / 0.072)^0.105)`.
- **Mass from `Msini`** (FUN_1404ca460): `Msini / sin i` with the orbit's stored inclination,
  which SE draws when the catalog omits it (see system_gen.md). Proxima b and d get SE's 1.4435
  and 0.35017 Earth masses.

## Bulk composition (FUN_140503b60, FUN_1401aa500, FUN_1401aaa10)

Seeded with the body's catalog seeds (two stream-A draws for the unset metallicities, one for a
10% "hot" flag). Bodies with M >= 60 and R >= 40000 km (Jupiters), or M >= 10 and R >= 18000 km (Neptunes), are
giants whatever their class (catalog bodies use these fixed thresholds); a catalog Jupiter or
Neptune below them is left alone in the port. A giant's heavy part (FUN_1401aa900) is ice
`tn(0.75, 0.02, 0.65, 0.85)`, rock `tn(0.15, 0.01, 0.05, 0.25)`, iron `tn(0.05, 0.005, 0.01, 0.1)`
and carbon `tn(0.35, 0.05, 0.05, 0.55) sat((C/O - 0.8) 5)`, normalised and scaled by
`tn(0.4, 0.2, 0.05, 0.8)` (Jupiter, FUN_1401aa7a0) or `tn(0.85, 0.2, 0.6, 0.95)` (Neptune,
FUN_1401aa850); the rest is H2 and `tn(0.085, 0.002, 0.075, 0.11)` of it He. For the rest, with `dens = sat(rho / 6 g cm^-3)` and `T` = `T_eq` if at
least 600 K, else 0:

- iron `tn(0.15, 0.1, 0, 0.9)`, silicate `tn(0.6, 0.2, 0, 1)`, carbon `tn(0.6, 0.2, 0, 1)`, ice
  `tn(0.15, 0.1, 0, 1)` (stream B);
- silicate and carbon x`(1 - 0.9 sat(dens - 0.8))`; carbon also x`sat((T-200)/400)^2 *
  sat((met - 0.8) * 5)` (0 for catalog stars); ice x`(1 - sat((T-100)/50)) * (1 - 0.999
  sat(dens - 0.3))`;
- normalise. Then the catalog class wins: Ferria/Terra/Carbonia cap ice below 10% and swap the
  class's component to the top; Aquaria raises ice to at least 10%.

## Spin, tides and heat (FUN_140503780, moons FUN_140502eb0)

SE's system step runs these for every catalog planet, dwarf planet, asteroid and comet
(FUN_140503780) and moon (FUN_140502eb0), with its catalog flag set (register R12 = 1 at both
calls): no asteroid-belt draw for dwarf planets and no cap on tidal heating. Stream order matters
for what follows, so the port replays it: the equator's node
`gauss(0, 5)` when the catalog has no `EquatorAscNode`/`EqAscendNode`, then the obliquity
`gauss(0, 40)` (one stream-A draw: `gauss(0, 180)` one time in five) plus the orbit's inclination
when it has no `Obliquity` (degrees, stored as radians), moment of inertia `tn(I0, 0.002, 0.28, 0.4)` with
`I0 = 0.4 - 0.07 sat((log10 M + 2.5) / 1.5)`, tidal Q `tn(600, 100, 200, 1000)`; giants
`tn(0.22, 0.04, 0.18, 0.3)` and `tn(2e5, 5e4, 2e4, 4e6) / tn(0.4, 0.05, 0.1, 0.6)`. A moon instead
first draws the orbit angles its catalog leaves out (inclination U(-1.5, 1.5) deg, node
U(0, 360) deg, system_gen.md) and takes its orbit's inclination as its obliquity; no node or
obliquity draws.

- Primordial spin `P0 = sqrt(I / M) * 1.11 R / 6371` days (giants: M at most 317.83). Despin time
  `|1/P0 - 1/P_orb| * 1.5 q^2 M / R_e^3 * I * Q` with `q = a_AU^3 / M_host`. Older than that:
  locked, or with chance rising with eccentricity a 3:2 (e > 0.04), 2:1, 5:2 or 3:1 resonance
  (FUN_140509e60, one stream-A draw). As the spin slows the axis turns toward the orbit's pole
  (obliquity moved by `age / despin` of the way to the orbit's inclination); a locked or resonant
  body, and one the catalog marks `TidalLocked`, takes the orbit's inclination (SE's panel then
  shows an axial tilt of 0).
- Tidal heating `e^2 a^-7.5 M_host^2.5 G^1.5 (1.05e17 / Q) R_km^5` W (FUN_1405099c0), tapered
  to 0 between 300 and 150 km radius. Procedural planets (FUN_1404fc280, flag clear) cap it at
  `(T/32)^4 * M_kg * 5e-12` with T = 50 K (500 K above 200 K) and damp the eccentricity; catalog
  bodies do not. Europa and Io get 30-200x Earth's internal heat.
- Volcanism (FUN_140512d50): `v = max(0, (log10 H - 9) / 2) + max(0, log10(s / age) + 1)` with
  `s = sqrt(M) * 5.49 - 0.45`; activity `clamp(v^2 / 2, 0, 2)`, 0 for giants. Internal heat
  `(gauss(1, 0.1) K + H + young) / K`, `K = M * 5e-12 * 5.97e24`, adds `heat * 2^20` to T^4.

## Atmosphere (FUN_1404494a0, FUN_1401aea60)

Bodies of 180 km radius or less stay airless. Otherwise, with g in m/s^2:

1. `vf = 10^tn(-5, 2, -10, -3)`, and for M > 0.5 `vf = min((2M)^2 vf, 5e-4)`. Pressure scale
   `1.25e6 M^2 / (R / 6378.14)^4`; `P = vf * scale`; inventory `vf M / 8.7073e-7`.
2. Initial inventory (FUN_1401aeda0): each species a cold body keeps (T < 1.2 triple T), plus
   `10^U(-1, 1) * outgassed * 1e6` (21 stream-A draws).
3. Loss with age (FUN_1401b1380): `(1 / (1 + rate))^((P + 0.75) age / 2)`; O2 and CO2 have
   variants for warm (270-400 K) bodies older than 2 Gyr. The pressure scales with what is left.
4. Jeans escape (FUN_1401b1a00): `v^2 = 24943.41 T_exo / mu`, `lambda = 3000 g R_km / v^2`;
   for lambda < 80 the species decays as `exp(-age / tau)`,
   `tau = e^lambda v^3 / (2000 g^2 R_km) / 3.1557e16` Gyr.
5. Climate solve (FUN_1401afb00), up to 50 damped (0.45) iterations to 1%:
   - phase split per species (FUN_1401b0370): boiling point from Clausius-Clapeyron, phase from
     the triple and critical points, gas share `(0.01 + 0.99 frac) * sat(Pvap / Ppartial)`;
   - vapour pressures (FUN_1401b2860): H2O (Murphy-Koop-like below 273 K, two Antoine fits
     above), CO2, NH3 and CH4 have their own fits; the rest use the table's Antoine values as
     `10^(A - B/(T + C))` Pa. Those values are really ln(mmHg) fits, so SE's generic gases are
     far too volatile (N2 boils at 43 atm); the port keeps this;
   - ocean fraction `sat(depth * 0.71 / 2.72)`, clouds from condensing species over the ocean,
     ice cover when liquid water exists;
   - albedo: clouds 0.52, land 0.15 (0.6 water-rich), ocean 0.04, ice 0.7 (thin atmospheres:
     land 0.07, ice 0.43), blended to 0.3 between 100 and 1000 atm; skipped when the catalog
     gives a Bond albedo;
   - `T = min(((1 - A) 278^4 flux + heat 2^20)^0.25, 3500) + greenhouse`, greenhouse
     (FUN_1401b0c60) `((k 0.75 + 1)^0.25 - 1) 0.43 P^0.4 T` with k by molar mass (3 / 2.34 / 1 /
     0.15 / 0.05 / 0 at 10 / 20 / 30 / 45 / 100) and x1.5 .. x8.333 by pressure.
6. Ocean depth `5 * liquid / (rho 4 pi R^2) / 2.72` km, capped where the pressure reaches 1 GPa;
   below 0.01 km the liquids are dropped. Water-rich bodies (no gas, >= 10% ice, ice layer
   > 25 km) get a global water ocean or a steam atmosphere instead.

Atmospheres under 1e-9 atm are removed. Above 1000 atm, or with steam, the body becomes a
Neptune (or Jupiter with > 25% gas) with the pressure capped at 1 atm (FUN_1404cabf0); a giant
without a catalog class gets the same choice.

Giants (FUN_140449860, FUN_1401ae280) take a gas share `s` of the mass: `x = (log10 M - 1)^3`,
`s0 = 0.05 + 0.9 x` (0.05 below 0, 0.95 above 1), `s = clamp((0.95 + 0.1 U) s0, 0.05, 0.95)` (one
stream-A draw). The pressure at depth is `min(1.25e6 M^2 / (R / 6378.14)^4 s, 1e6)` atm and the
temperature there `min(T (P)^0.28, 5000)`. Gases start from the species' nebular columns (fields 13
/ 15, mixed by C/O) times `1.15^U(-1, 1)` (21 stream-A draws), lose to Jeans escape and are
rescaled to `s M / 8.7073e-7`. The model's surface is the 1 atm level (no greenhouse, density at
the deep temperature); SE's panel shows the deep pressure. The visual atmosphere height of giants
(FUN_140448ef0) is `clamp(15.6 T_eq R_gas / (g mu), 100 km, 5% R)`.

## Ocean, bump height, clouds (FUN_1405057e0)

`BumpHeight` defaults to `min(20 / sqrt(g / g_earth), 20, 0.05 R)` km (0.9 R for asteroids); this
is SE's default for every catalog body without one. A liquid inventory makes an ocean whose depth
is the solve's; when it is at least the bump height and holds water, the bump becomes
`depth + 0.5 + U * 1.5` km. Clouds form above 0.05 atm. The volatiles class then follows from the
depth against the bump (FUN_140505680).

## Look (FUN_140448500)

Seeded with the body's seed A + 0x8535f; the fourth stream-A value `u` picks the model:
biogenic with life below u = 0.25; Chlorine (or Thick) above 0.1% Cl2; Neptune or Titan above 0.2%
CH4; otherwise by pressure and arid/wet, e.g. arid 0.001-0.05 atm: Thin, Mars from u = 0.4,
Jupiter from 0.8; wet 0.05-15 atm: Earth (Titan over hydrocarbon seas), Jupiter from 0.9; below
0.001 atm Ethereal or Pluto. `Bright = clamp(10 (10 P)^0.2, 0.5, 10)`, `SkyLight = Bright / 3`,
Mars and Pluto models get 1.5x the height. Catalog systems keep hue 0 and saturation 1. Giants
(class Neptune and up) take Earth's model, Jupiter's from u = 0.75, `Bright` 3, `SkyLight` 1 and
an opacity of 1, falling linearly to 0.2 from 500 to 1500 K.

## Temperature model and the class word (FUN_14025f1c0)

Once a system is generated (FUN_140510f40), every body gets a temperature model at record
+0x184c, and the temperature class (+0x28, the first word of "Cold arid terra") is reset from its
average (+0x1860) with the usual thresholds (> 1000 Torrid, > 500 Hot, > 330 Warm, > 250
Temperate, > 170 Cool, > 90 Cold). The surface generator and preset choice run after that, so they
see this class too, not the one from the surface temperature (+0x1250). Ported as
`climate::class_temperature`. Inputs: mode-2 flux `F`, Bond albedo `A`, pressure `P`, air density
`rho`, internal heat, greenhouse, obliquity, rotation period, and "locked to a star" (record
+0x165c bit 2: locked, bulk class below Neptune, parent a star or a barycentre of stars).

- `T(f) = (F * 1370 (1 - A) / (f sigma))^1/4`, with `f` 2 for a locked planet, 4 from 0.2 atm,
  else 1. The model's own copy carries a 3.826e26/3.846e26 luminosity round trip.
- Mixing term `h`: 20, or `|10 - 2 ln rho|` from 0.01 atm (5 under a crushing or steam envelope
  and for giants), 66.67 when locked; used as `h/100`.
- Night/day ratio `r` (FUN_14025e9b0): airless (P <= 0.001): night = heat and greenhouse alone,
  day = `T(f)` with f from the pressure; with air the day/night swing is
  `max(5, (25 - 11 log10(1.01325 P)) * 2 / P_rot_days)` (at most 0.99 T); giants 0.75. `r =
  min(night/day / (1 - h/100), 0.99 when above 1)`, both floored at 20 K.
- Average before heat (FUN_14025df00): locked, `0.525 T(2)` (the night side at a third of the
  day side, FUN_14025cc40; its wind terms cancel); thin air or airless, `T(f) * |cos(L / (1 + 2
  |obl| / pi)) (1 - r) + r| * (0.463 h/100 + 1 - h/100)` with `L` 52.5 deg with air, 41 deg
  without (FUN_14025cf40); from 0.2 atm `T(4)` is first divided by `|cos(57 deg) (1 - r) + r|`.
  Giants and crushing atmospheres use 60 deg and an optical-depth factor (FUN_14025cd30).
- Average = `(T^4 + heat * 2^20)^1/4 + greenhouse`, at least 20 K.

The info panel's "Average temperature" is not this value: the panel reruns the model for the
stars' current distances (FUN_14025a220, panel_temperature.md).

## Checked against SE (2026-10-27)

SE's info panel for Proxima b: "Cold arid terra", 1.4435 Earth masses, age 4.85 Gyr, locked 1:1,
atmosphere CO2, SO2, N2, H2 at 0.163 atm, average temperature -130 C. Proxima d: "Cool lacustrine
terra", water seas, CO2, N2, H2O, SO2 at 1.511 atm, -85 C. Both catalog entries give only
`AlbedoGeom 0.367`, which SE uses as a fixed Bond albedo. With SE's masses the port gives
Proxima b 0.162 atm of CO2, SO2, N2, H2 (62/32/4.5/0.6%), arid, 230 K at the surface, average
144 K, "Cold"; Proxima d 1.505 atm of CO2, N2, H2O, SO2 (86/7.6/4.4/1.9%), lacustrine, 301 K,
average 190 K, "Cool" (ignored test `proxima_planets_complete_like_se`). The surface
temperatures agree with SE's Earth Similarity Index (0.771 and 0.862 invert to 229 and 301 K).
The app derives the same masses from `Msini` (system_gen.md) and gets 0.162 and 1.505 atm.

51 Pegasi b (Dimidium, `Msini` only): SE's panel "Torrid subjupiter", 0.47171 Jupiter masses, H2 and
He, 1.297e5 atm; the port gives 149.9225 Earth masses (0.47171), H2 90% and He 9.4%, 1.2975e5 atm
at depth, "Torrid", Jupiter. Saturn's Tethys and Hyperion, which the catalog leaves without
atmospheres: SE "Frigid airless microaquaria" and "Cold asteroid", the port Frigid / airless /
Aquaria and Cold / Asteroid (ignored test `panel_temperatures_match_se`).

The class word matches SE's panel for Mercury (Warm), Venus (Hot), Earth and the Moon
(Temperate), Mars and Ceres (Cool), Io, Europa, Ganymede, Callisto and Titan (Cold); the ignored
test `solar_system_class_words_match_se` pins these.

Without a catalog albedo the solve is bistable: if condensing gases liquefy on the first passes,
clouds raise the albedo to ~0.6 and the planet settles cold, so such bodies depend on every random
draw before the atmosphere matching SE's. The draw order (FUN_140503780: reseed from the body's
seeds, node, obliquity, moment of inertia, Q, spin evolution unless locked, tides, heat, then
the atmosphere) is the port's; the Proxima results above confirm it for those two planets.

## Not decoded / approximations

- Bodies the catalog describes fully get Earth's internal heat plus, for moons, tidal heating with
  SE's mean tidal Q (600; SE draws it per body), and air of molar mass 29 for the density term.
  Their obliquity is the catalog's, or for a moon its orbit's inclination; a catalog planet without
  one keeps 0 (SE draws it).
- A moon whose planet is a binary component (parent type 1 or 2, FUN_14050f880) uses the
  barycentre's orbit in SE; the port uses its own.
- The system age without a catalog `Age` follows SE's first stream-A draw from the position seed (docs/formats/system_gen.md); not yet compared with an SE panel.
- Life generation, aurorae and SE's ocean/cloud layer colours are not ported.
