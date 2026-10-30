# Catalog systems: seed, generated orbit angles, Msini masses

When SE builds the planetary system of a catalog star (FUN_140510f40 -> FUN_1404f62e0) it fills
in orbit angles the catalog leaves out and derives masses from `Msini`. Ported as
`se_core::system_gen` (`complete_catalog_orbits`, run at catalog load).

## System seed (FUN_140510f40)

Logged as `Creating planetary system of star "...", seed = N` (in
`Documents/Cosmographic/SpaceEngine/logs/se.log`). Both random streams (thread-local A at +0x24,
B at +0x20, LCG `s * 0x19660d + 0x3c6ef35f`) start from it.

- The star loader (FUN_140476e90) stores the root star's or barycentre's position as f32
  parsecs, universe frame, Sun at the origin: RA (`h + m/60 + s/3600`) times pi/12 and Dec times
  pi/180 in double, cast to f32; distance `Dist`, `DistLy * 0.3066011287849475` or
  `1000 / Parallax` (later keys win), cast to f32; then in f32
  `x = sin(ra) * d * cos(dec)`, `y = sin(dec) * d`, `z = cos(ra) * d * cos(dec)`.
- Each coordinate is read as `floor(v * 2^32)` modulo 2^32 (the middle 32 bits of the 128-bit
  position) and `seed = x * 0x28842 - z * 0x2309fb + y * 0xe35ad` (wrapping 32-bit).
- Checked: Proxima Cen 1983888000, Sirius -308782848, Solar System 0.

## Draws before the angles (FUN_1404f62e0)

With no `Age` on any record of the system, the first stream-A draw sets the system age: `u` times
the shortest main-sequence lifetime of its stars, at most 12.87 Gyr (`system_gen::age_draw`,
`climate::catalog_system_age`). The other helpers that draw before the angle step save and
restore both streams.

SE lists a system's records from the root depth first, each record followed by its children in
link order (FUN_1404fab30), and puts the stars and barycentres first in that order; the angle step
runs over them in it.

## Orbit angles (FUN_14050d340)

Run for each star record (barycentres included) with children, in record order (above). Children are
linked by type rank (star, barycentre, planet, dwarf planet, moon, ...; table at 0x14067a748),
ties in catalog order. An `Orbit` block without `Inclination` sets flag 0x1000; without
`AscendingNode` and `PositionAngle`, flag 0x2000.

- Statistics over children that are stars, barycentres, planets or dwarf planets with an orbit
  and a known inclination: mean and minimum of the inclinations and of the ascending nodes (the
  node ones too, even where only the inclination is known). Largest `Msini` of the planets and
  dwarf planets.
- None known: `lo = asin(max Msini / 80 M_J) + 3 deg`; mean inclination uniform in
  [lo, 90 deg) and mean node uniform in [lo, 180 deg) (one stream-A draw each); sigma 1 deg.
- Known: the means above; sigma is the variance of the known values around the mean (rad^2,
  not its root, divided by the count); if that is 0, sigma 1 deg and `lo = min + 3 deg`, else
  `lo = min`.
- Each planet or dwarf planet missing an angle draws it from a normal (mean, sigma) truncated to
  [lo, 90 deg] for the inclination and [lo, 180 deg] for the node: one stream-B draw `u`, then
  `mean + sigma * ndtri(((erf(b) - erf(a)) * u + erf(a) + 1) / 2)` with
  `a = (lo - mean) / (sqrt 2 sigma)`, `b = (hi - mean) / (sqrt 2 sigma)` (FUN_140441b80, Cephes `ndtri`
  FUN_140444dc0). With `lo` above `hi` the draw lands just past `lo`.

## Moons (FUN_140502eb0)

A catalog moon whose `Orbit` omits angles draws them first in its own completion, on stream A
seeded with its own seed (record +0x48, `surface_gen::catalog_seeds`): inclination
`(3 U - 1.5) deg`, then node `360 U deg`, each only when missing (`system_gen::moon_angles`, applied
by `complete_catalog_orbits`).

## Mass from Msini (FUN_1404ca460)

`PlanetMsiniResolve 0`: mass = `Msini / sin(i)` with the stored inclination, capped at 80 Jupiter
masses (25426.27 Earth masses). Mode 1 uses `Msini * sqrt 2`, mode 2 the `Msini` itself.

## Missing argument of pericentre and mean anomaly (FUN_1401cc2e0)

SE's `Orbit` loader treats the longitude of pericentre (`LongOfPericen`) and the mean longitude
(`MeanLongitude`) as 0 when absent: without `ArgOfPericenter` (or `ArgOfPericen`,
`ArgOfPeriapsis`) the argument is `LongOfPericen - node`, and without `MeanAnomaly` the mean
anomaly is `MeanLongitude - (w + node)`, at `Epoch` (J2000 when absent). The node is the
catalog's (`AscendingNode`, else `PositionAngle`, else 0, as the record constructor FUN_1400243f0
zeroes it); a node drawn later by the angle step changes neither. Proxima b (`ArgOfPericen 290`
only) starts at M = -290 = 70 deg at J2000. Checked in SE: Proxima b's distance from a camera
1e6 km from Proxima at eight dates over one orbit (2000.01.01 12:00 + k P/8) fits M = 70.0 deg to
17 km (with the camera 1e6 km from the star; the fit's minimum is sharp, 250 km at 69 and 71 deg),
and the panel temperature then matches (143.852 K, SE 143.850). Ported in `KeplerOrbit::from_block`.

## Checked against SE

Proxima c gives `Inclination 133`, `AscendingNode 135`: Proxima d and b (linked before it) draw
131.98 and 132.10 deg, so `0.26030351 / sin i` and `1.071 / sin i` give 0.350170 and 1.44349
Earth masses, SE's panel values (0.35017, 1.4435). 51 Pegasi b (Dimidium), the only planet of its
star and without an inclination, takes the stream-A branch: the port gives 0.47171 Jupiter masses,
SE's panel value. Tests `proxima_planets_get_se_inclinations_and_masses` and the ignored
`proxima_masses_from_catalog_match_se`.

## Not confirmed

- The frame the angles orient the orbit in: the port keeps its ecliptic default for these
  orbits, so the generated planes match SE's values but not necessarily its on-screen tilt.
