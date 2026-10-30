# Analytic moon theories

SpaceEngine positions the moons of the major planets with analytic theories instead of the
Keplerian elements in the catalog. The catalog names the theory in the moon's orbit block:

```
Moon "Io/Jupiter I" { Orbit { AnalyticModel "Io-L1" ... } }
```

The Keplerian keys next to `AnalyticModel` (period, semi-major axis, ...) are only used for the
orbit line and as a fallback. The theories are C++ classes in `system/SpaceEngine.exe`
(build 0.991.49.2095 beta, image base `0x140000000`); their coefficient tables are statically
initialised objects in `.data`. Nothing ships in the paks (`data/ephemerides` holds only
`DE436.pak`). The port reads the tables from the user's own executable at runtime
(`crates/se_core/src/moon_theories/`, `exe.rs` maps the PE image by virtual address); no table
is copied into the repository, and a theory whose sentinel check fails is skipped, which leaves
that moon on its Keplerian orbit.

## Models used by the catalogs

| `AnalyticModel` | Class (RTTI) | Moons | Parent |
|---|---|---|---|
| `X-L1` | `seOrbitL1` | Io, Europa, Ganymede, Callisto | Jupiter |
| `X-TASS17` | `seOrbitTASS` | Mimas, Enceladus, Tethys, Dione, Rhea, Titan, Hyperion, Iapetus | Saturn |
| `X-GUST86` | `seOrbitGUST86` | Miranda, Ariel, Umbriel, Titania, Oberon | Uranus |
| `X-NOE-4` | `seOrbitNOE4` | Phobos, Deimos | Mars |
| `X-HTC20` | `seOrbitHTC20` | Helene, Telesto, Calypso | Saturn |
| `X-Noyelles` | `seOrbitNoy` | Janus, Epimetheus | Saturn |
| `X-Emelyanov` | `seOrbitNEP` | Triton | Neptune |

(`catalogs/planets/SolarSys.sc` and `Moons-small.sc`.) The `X-DE` models (planets, Earth, Moon,
Sun) are the JPL DE436 path in `jpl_de.rs`. `seOrbitVSOP87Rect/Polar`, `DEA` and `JPL` are
registered by the same code but no catalog entry selects them. Other moons (Charon, Pluto's
moons, Neptune's small moons, ...) have no analytic model and stay Keplerian.

## Where things are

All addresses are virtual addresses in the 0.991.49.2095 executable.

* Registration: `FUN_1401ea260` builds one 0x388-byte registry entry per body with `FUN_1401eb380`
  (theory name, short name, body name, pointer to the static orbit object, flags, optional GM
  override; the moon entries use none).
* Orbit objects (vtable at +0; `+0x70` = GM in AU^3/day^2 where used):

| Theory | Vtable | Objects (stride) | Evaluation |
|---|---|---|---|
| GUST86 | `0x140689128` | `0x14079b880` (0x100), 5 | `FUN_1401e6060`, series `FUN_1401e6470` |
| L1 | `0x140689290` | `0x14079f6a0` (0x110), 4 | `FUN_1401e8160`, elements `FUN_1401e8370` |
| TASS | `0x140689588` | `0x1407ac3c0` (0x120), 8 | `FUN_1401edc30`, elements `FUN_1401edef0` |
| NOE-4 | `0x1406892e0` | `0x1407a10c0` (0x110), 2 | `FUN_1401e9990`, `FUN_1401e9f00`, `FUN_1401e9ad0` |
| HTC20 | `0x1406802d8` | `0x140763870` (0xd0), 3: Helene, Telesto, Calypso | `FUN_1401772d0`, `FUN_1401773f0` |
| Noyelles | `0x140680300` | `0x140764100` (0x130), 2 | `FUN_1401778c0`, `FUN_1401779e0`, `FUN_140177bf0` |
| NEP (Triton) | `0x1406892b8` | `0x14079fc00`, 1 | `FUN_1401e8e60`, `FUN_1401e9030` |

  Vtable slots: 1 = init (builds the frame matrix), 2 = position, 3 = velocity.
  Note that the registration order for HTC20 is Helene, Calypso, Telesto but the objects sit in
  memory as Helene, Telesto, Calypso.
* Shared: `FUN_1401ec3f0` (elements to position and velocity), `FUN_140022320` (3x3 product),
  `FUN_1400050b0` + `0x140005800` (global frame matrix `DAT_140854f90`), `FUN_1401e3210` (epochs).
* Tables, as laid out in the objects:
  * GUST86: `+0x78` points to three (pointer, count) vectors of 24-byte terms (ten signed-byte
    multipliers for the five `N` and five `E` arguments, amplitude at +0x10), `+0x80` to the five
    inclination amplitudes, `+0x88` to seven doubles `[dN/dt, dE/dt, dI/dt, N0, E0, I0, n0]`.
  * L1: `+0x80` points to four (pointer, count) vectors of `{amp, phase, freq}` triples, `+0x88` to 45
    Chebyshev coefficients (nine per element, interleaved), `+0x90` to six mean-element offsets,
    `+0x78` is the mean motion (rad/day).
  * TASS: `+0x80` six mean elements, `+0x88` four vectors (n, lambda, z, zeta) of groups
    `{7 signed-byte multipliers, pointer, count}` whose terms are `{amp, phase, freq}`;
    `+0x78` is `n0`, `+0x90` the argument index (7 for Hyperion).
  * NOE-4: `+0x78`, `+0x80` mean motion and its rate, `+0x88` six constants (`a` AU, lambda0, k, h, q, p).
  * HTC20: `+0x78` four frequencies and four phases, `+0x80` / `+0x88` 56-byte terms and their count.
  * Noyelles: `+0x78` ten doubles (five frequencies, five phases), four term vectors at
    `+0x80/+0x88/+0x90/+0x98` with counts at `+0xa0/+0xa4/+0xa8/+0xac`, `+0xb0` a longitude offset.
  * NEP: `+0x88` six constants, `+0x90` seven 32-byte terms.

## Time

Positions take a 128-bit fixed-point time (days x 2^80, the same format as saved places). Each
theory subtracts its own epoch and works in days (or years) of TDB:

| Theory | Epoch (JD) | Argument |
|---|---|---|
| GUST86 | 2444239.5 | days |
| L1 | 2433282.5 | days |
| TASS | 2444240.0 | days |
| NOE-4 | 2451545.0 | days; mean elements at +6491.5 d |
| HTC20 | 2451545.0 | days |
| Noyelles | 2451545.0 | Julian years |
| NEP | 2378520.5 (node, longitude) and 2451545.0 (arguments) | days |

Validity (set by `init`, used by the registry entry to switch to a Keplerian continuation outside):
L1 JD 2287196 to 2579380 (years 1550 to 2350), NOE-4 625308 to 4277758, Noyelles 2433283 to
2469808 (1950 to 2050); the others are unbounded. The port returns `None` outside these ranges
and `Ephemeris` falls back to the catalog orbit (SpaceEngine continues the Kepler orbit from the
boundary state; that continuation is not ported).

## Frames and units

Every theory forms planet-centred Laskar elements `(a or n, lambda, k, h, q, p)`, solves
`F - k sin F + h cos F = lambda` by Newton iteration (at most 10 steps, 1e-14) and builds the
position with the standard `x1, y1` plus `(q, p)` rotation (`FUN_1401ec3f0`). `a` comes from `n`
by `cbrt(GM / n^2)` for GUST86 and TASS, and directly (AU) for L1, NOE-4 and Noyelles (km/AU).
GUST86 negates the third coordinate before the frame matrix.

The result is rotated by a per-moon matrix built in the init slot and multiplied by the global
matrix `DAT_140854f90`. All matrices are row-major and applied to row vectors,
`out_j = sum_i p_i M[3 i + j]`, and the products are
`Y(-pi/2) * Z(b) * Y(a) * G` for GUST86, L1 and Noyelles (angles read from the executable;
Y/Z are the rotation layouts used by the init slots), `Rx(c) * Ry(a)` without `G` for TASS,
`Y(-pi/2) * Z(delta(t)) * Y(alpha(t)) * G` for NOE-4 (IAU Mars pole, time dependent),
`Z * Y * G` plus the node/inclination rotation for Triton, and the identity for HTC20. `G` is the
transpose of the Euler matrix `(-epsilon, pi/2)` with `epsilon = 0.4090928040284034`.

Output positions are in parsecs (AU x 4.848136811095360e-6); the port keeps AU. The frame is
SpaceEngine's internal ecliptic frame, `(X, Y, Z) = (x_ecl, z_ecl, -y_ecl)` (J2000 ecliptic with
Y up), found by fitting the output against Horizons vectors and then confirmed by the planetary
pole matrices. The port converts with `ecliptic_to_universe(X, -Z, Y)` and multiplies by
`METERS_PER_AU`.

## Details of each theory

* **GUST86** (Laskar and Jacobson 1987). Arguments `N_i = fqn_i t + phn_i`, `E_i = fqe_i t + phe_i`,
  `I_i = fqi_i t + phi_i` for the five satellites (mod 2 pi). `n = n0 + sum A cos(m.(N,E))`,
  `lambda = fqn t + phn + sum A sin(...)`, `(k, h) = sum A (cos, sin)(...)`,
  `(q, p) = sum_i amp_i (cos, sin) I_i`.
* **L1.** `a`, `lambda`, `(k, h)`, `(q, p)` = sum of `amp cos/sin(freq t + phase)` + constants,
  then a Chebyshev correction of five elements (`a, lambda, k, h, q`; `p` has none),
  `x = (t / 182.625 + 7.0058316) / 1632.4494`, weight `min(1, (257501.25 - |t|) / 3652.5)`,
  and `lambda += t * n`. See *Quirks*.
* **TASS 1.7** (Vienne and Duriez 1995). Seven argument series `sum amp sin(f t + ph)` (taken from
  the first lambda group of Mimas, Enceladus, Tethys, Dione, Rhea, Titan, Iapetus) feed the
  multiplier-weighted phases of all four series (term phase `f t + ph + sum m_i arg_i`).
  `n = (1 + c0 + sum A cos(...)) * n0`; `lambda = c1 + arg_own + sum A sin(...) + t n0`
  (Hyperion has no own argument and keeps its first group); `(k, h)` and `(q, p)` start from the
  mean constants and add `A cos` / `A sin`.
* **NOE-4.** A fixed ellipse (constant `a, k, h, q, p`) with
  `lambda = lambda0 + (n + rate s) s`, `s = t + 6491.5`. The executable also evaluates four trigonometric
  series for these elements but then overwrites them with the constants, so only the mean ellipse
  has any effect. The frame follows the IAU Mars pole (RA 317.68143 - 0.1061 T, Dec 52.8865 - 0.0609 T).
* **HTC 2.0.** Direct Fourier series for Cartesian components (AU, ecliptic) in four arguments;
  stored as `(x, z, -y)`. A flag selects sine or cosine.
* **Noyelles.** `a = a0 + sum A cos`, `lambda = A_0 + offset + sum A sin`, `(k, h)`, `(q, p)` with
  two or three multipliers per term. The `zeta` pointer of Janus aliases the first two `z` terms
  (and Epimetheus has none), so its inclination is nonphysical.
* **Triton.** A circular orbit of radius 354758.98 km with inclination, longitude and node corrected by
  seven terms in two arguments: `N = 258.329018 + 0.00598182615 t` and `x = 200.788305 - node`
  (degrees), with `node = 72.89882654 + 0.001433819551 s`.

## How this was verified

The evaluation functions were read from the Ghidra project and every theory was then checked
against the executable itself by running the vtable position slots (after `init`) in an x86-64
emulator over the mapped image. The Rust port agrees with those outputs to about 1 m for all 25
moons at seven epochs between JD 2433300 and 2470000 (`matches_executable_output` pins a subset).
Independent checks against JPL Horizons vectors (geometric, TDB, ICRF, planet-centred) are in
`matches_horizons`.

## Accuracy against Horizons

Position error in km at JD 2451545.0 / 2458849.5 (SpaceEngine's theory as implemented in the
executable; Horizons is the reference):

| Moon | J2000 | 2020 | Remark |
|---|---|---|---|
| Tethys, Dione, Rhea, Titan, Enceladus | 130 to 230 | 95 to 220 | TASS 1.7 |
| Mimas | 2100 | 570 | |
| Iapetus | 800 | 800 | |
| Hyperion | 1800 | 5100 | chaotic rotator, theory is approximate |
| Io, Europa, Ganymede, Callisto | 3200, 7800, 11800, 12800 | 2800, 7600, 11400, 12600 | with the Chebyshev correction; 20 to 280 km without it |
| Ariel, Umbriel, Titania, Oberon | 900 to 4000 | 1100 to 4100 | GUST86; error is a 0.3 to 0.5 degree rotation about the pole |
| Miranda | 14600 | 780 | |
| Phobos, Deimos | 190, 310 | 290, 250 | |
| Telesto, Calypso, Helene | 2100, 8400, 38000 | 25000, 15600, 150000 | phase errors of several degrees, up to 23 for Helene |
| Janus, Epimetheus | 96000, 123000 | 111000, 80000 | 30 to 170 degrees of phase (the pair swaps orbits every four years) |
| Triton | 681000 | 681000 | near-constant 147 degree phase offset |

## Quirks

* **L1 Chebyshev block.** With the 45-coefficient correction, the Galilean moons' radii are 0.7 to
  1.2% off (Io -0.8%, Europa +1.2%, Ganymede +1.1%, Callisto -0.7%); without it L1 matches Horizons to a
  few hundred km. The port reproduces the executable (`MoonTheories::with_l1_chebyshev(false)` turns
  the correction off).
* **Triton, Janus, Epimetheus, Helene.** The executable's own output is far from Horizons (see the
  table); the port reproduces it rather than correcting it.
* **GUST86 at t = 0.** The argument cache (`DAT_140853648`) starts at 0.0, so the very first query at exactly
  JD 2444239.5 reuses unset arguments. Later queries are unaffected; the port always computes.
* **GUST86 frame.** The five moons share a roughly 0.3 to 0.5 degree in-plane offset (error
  proportional to the orbit radius, along-track). The executable's frame chain uses the IAU J2000
  Uranus pole; the published theory's longitude origin is probably referred to an older
  equinox, which would explain it.
