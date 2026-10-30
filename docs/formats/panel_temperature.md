# Panel temperatures: night, average, day (SpaceEngine 0.991)

SE's info panel prints a body's "Average temperature". It is computed as the panel draws
(FUN_14025a220, called from the UI with the observer's position), from SE's temperature model
(record +0x184c, set up by FUN_14025f1c0; its own average +0x1860 picks the class word, see
climate.md) and the current distance of each star. Ported as `se_core::climate::display`
(`catalog_model`, `catalog_view`, `Model::panel`); the port's selection window shows the average
with the night and day values.

## Model (FUN_14025f1c0, FUN_14025e9b0, FUN_140513830)

Floats of the model, besides the body record (+0x48) and its atmosphere (+0x50):

| Offset | Value |
|---|---|
| +0x00 | wind (m/s): `Climate { GlobalWindSpeed }`, else `10 (1/rho)^(1/3)` from 0.01 atm (divided by 10 until at most 200), or for giants and crushing atmospheres `100 (1/rho)^(1/4)` (at most 2000); at most 0.75 of the escape speed |
| +0x04 / +0x08 | `Climate { MaxSurfaceTemp / MinSurfaceTemp }` (Venus 741; Mars 98 and 308; Bennu) |
| +0x10 | substellar temperature: the catalog maximum, else `((1 - A) F 1370 / (f sigma))^1/4`, f = 4 from 0.2 atm, else 1 |
| +0x18 | optical depth tau: `rho 1000 h_0.2 wind / (2 sigma T^3 pi R)` clamped to 0.1 .. 20 (h_0.2 the height of the 0.2 atm level, 0 for thinner air), 0.1 below 0.01 atm; giants `1.78851 (e^(wind/1000) - 1)`, at least 0.1 |
| +0x1c | night/day ratio (climate.md), from the catalog minimum and maximum when given |
| +0x20 | vertical mixing h (percent): 20, `|10 - 2 ln rho|` from 0.01 atm, 5 for giants, 66.67 when locked |
| +0x30 / +0x34 / +0x38 | periapsis, mean `a (1 + e^2/2)` and apoapsis distance (pc) of the orbit around the star; a moon's adds its own orbit to its planet's (`q_p - Q_m`, sum of means, `Q_m + Q_p`) |

## One star (FUN_14025e400)

`T = ((L 3.826e26 W / (4 pi d^2)) (1 - A) / (f sigma))^1/4` with `d` the current distance (pc to
metres as SE converts it) and `L` the bolometric luminosity; with f = 4 divided by
`|cos 57 deg (1 - r) + r|`; 20 K when zero. For the body's own star with catalog extremes the
light is ignored: the maximum (without heat and greenhouse, FUN_14025e2e0) scales as
`sqrt(q / d)`; the minimum, divided by `|cos(L_n) (1 - r) + r| (1 - h)` with `L_n` the night-side
latitude `(|obliquity| + 90 deg) / (1 + 2 |obliquity| / pi)`, as `sqrt(Q / d)` (times 3 for a
locked body); with both on an eccentric orbit `T = a / sqrt(d) + b` passes through the maximum at
periapsis and the scaled minimum at apoapsis.

Each star adds night, day and average values as T^4, by regime:

- **Rotating, thin or no air** (FUN_14025be70, f = 4 from 0.2 atm else 1). The day follows a
  parabola in hour angle peaking at `min(1, t(tau))`; after sunset `T^-3` grows by `6 pi / tau` per
  radian (FUN_14025d830). With the night floor `T_f` (a full turn of cooling) and the mixing,
  `day = ((peak - T_f) / (1 - T_f) h + 1 - h) T`; night uses the pole turned away (dark through a
  season, half lit at an equinox) times the night-side latitude factor; average = FUN_14025cf40,
  the same latitude-averaged term as the class temperature (41 deg airless, 52.5 deg with air). When
  that average beats the mean of day and sunset at the subsolar latitude (and the star stands within
  45 deg of the equator) night and day are scaled to it.
- **Giant or crushing atmosphere** (FUN_14025b430, f = 4): day = the peak factor (with the
  optical-depth factor 0.95 .. 1) times T; night from the far pole and FUN_14025d170; average =
  FUN_14025cd30 (latitude 60 deg, wind term).
- **Locked to its star** (FUN_14025aef0, f = 2): day = `c T`, night `0.9 c T q (1 + 2 (1 - c))`
  with `q = 1.1 T_min / T`, `T_min` the catalog minimum or `T / 3`, `c` the optical-depth factor;
  average = FUN_14025cc40 (day and night sides).

Night, average and day each become `(sum T^4 + heat 2^20)^1/4 + greenhouse` (FUN_14025e390), at
least ordered night <= average <= day. With catalog extremes (FUN_14025aa60) the model's night ..
day range is mapped onto `[max(min, scaled min), min(max, sqrt(q/d) max)]` (only one given: all
three scale by it), then clamped to the extremes.

## Checked against SE (2000.01.01 12:00)

| Body | SE | Port |
|---|---|---|
| Mars (catalog min/max) | 236.632 K | 236.629 K |
| Ceres | 177.341 K | 177.342 K |
| Tethys (completed moon) | 75.69 K | 75.72 K |
| Hyperion (completed moon) | 103.44 K | 103.54 K |
| Moon | 276.734 K | 276.539 K |
| Mercury | 402.58 K | 402.87 K |
| Proxima b (locked) | 143.85 K | 143.85 K |
| 51 Peg b (giant) | 1239.01 K | 1242.81 K |

Mars at 2026.07.22 04:04 gave 223.008 K in SE. Ignored test `panel_temperatures_match_se`.

The Moon fits only with SE's obliquity rule for catalog moons: one without an `Obliquity` takes its
orbit's inclination (FUN_140502eb0 runs on every catalog moon), 5.145 deg for the Moon.

## Not confirmed / not ported

- Mercury and the Moon are 0.07% off. Not the internal heat: running SE's completion draw
  (gauss(1, 0.1) of Earth's per mass) for them changes nothing at this precision, and closing the
  gap would take some 70 (Mercury) or 16 (Moon) times Earth's heat. More likely the bodies'
  distances (SE's ephemerides) or albedos. The Moon's main star is the Sun through the Earth-Moon
  barycentre (`year_orbit`); that did not move it either.
- 51 Peg b is 0.3% off; its eccentricity (0.0042) cannot explain that. (Proxima b matched once its
  orbit took SE's default mean anomaly, system_gen.md.)
- The latitude under each star comes from the port's spin axis, not SE's (only night and day
  depend on it).
- Not ported: the local temperature at the observer and at altitude (FUN_140259df0, `AtmoProfile`).

## Day lines (FUN_140515cf0, FUN_14025de00, FUN_14025db80)

SE's panel strings "Solar day", "Daylight hours at equator/current latitude" and "Annual fraction
of polar day at current latitude". Ported as `display::day_lines`, shown in the selection window
for the latitude under the camera.

- Solar day (FUN_140515cf0): the sidereal rotation `P` and the year `Y` (the orbit around the star
  of the body, or of the planet or barycentre it goes around: the Moon's year is Earth-Moon's),
  `P Y / (Y - P)` when spin and orbit turn the same way (SE: obliquity and the orbit's inclination
  both under 90 deg), else `P Y / (Y + P)`, rounded to 1e-6 days. Earth 24.000 h, Venus 116.751 d,
  the Moon 29.531 d (a moon without a rotation turns once per orbit). None when `P = Y`.
- Daylight share (FUN_14025de00): `acos(-tan(lat) tan(decl)) / pi` (1 or 0 past +-1), `decl` the
  latitude under the main star; the equator line is always half a day. Hours = share x solar day.
- Polar day (FUN_14025db80): over 6283 steps `t = 0.001 k` of a year the star stands at
  `tilt sin t` (tilt: the spin axis from the orbit's pole, folded into +-90 deg); the percent of
  steps with `tan(decl) tan(lat) <= -1` (the same count as polar day by symmetry).
- For a body locked to its star SE stores other values (100 for the polar line); the port shows
  no day lines there.
