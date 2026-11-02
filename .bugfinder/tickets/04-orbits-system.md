# 04 Orbits, rotation, ephemerides, time and the active star system

## Paths in scope
- crates/se_engine/src/system.rs (1.8k lines: picking the active system, spawning/despawning bodies,
  computing positions each frame)
- crates/se_core/src/orbit.rs, rotation.rs, ephemeris.rs, jpl_de.rs, time.rs, units.rs, star.rs,
  body_params.rs
- crates/se_core/src/moon_theories/* (analytic moon theories; tests.rs for reference)

## What it does
Computes where every body is at the current simulation time (Kepler orbits, JPL DE ephemeris for the
solar system, analytic theories for big moons), how it is rotated (rotation period, obliquity,
precession, tidal locking), and the time system (Julian dates, time rate, pause). system.rs picks
which star system is "active" and spawns its bodies.

## Likely bug classes
- Wrong results: Kepler solver not converging for high eccentricity (e≈1, hyperbolic e>1), wrong
  anomaly at the epoch, degrees/radians, epoch/time scale (TT vs UTC vs TDB), J2000 frame vs ecliptic,
  wrong parent for a body in a barycentre hierarchy, retrograde orbits/rotation sign.
- Panics or NaN: dates outside the ephemeris file range, zero period, zero mass, negative time rate,
  reversed time.
- Leaks/desync: entities not despawned when the active system changes, two systems active at once,
  flip-flopping between systems at a boundary (hysteresis), bodies of the old system lingering.
- Time: accumulated f64 precision loss at huge time rates, time jumping when unpausing.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. `cargo test -p se_core` covers moon theories; scratch crates can call
orbit/ephemeris code directly. Port runs: `date`, `select`, `goto`.
