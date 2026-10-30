# Bugs found while porting (not in the audit)

- ~~**Name lookup ignores case**~~ Fixed 2026-10-30: names two objects share but for case
  ("Kepler-34 b" the planet, "Kepler-34 B" the star) are also kept exactly as written, and
  `Catalog::find` tries those first.
- ~~**'Oumuamua sits at the Sun's centre**~~ Fixed 2026-10-30: SE writes hyperbolic orbits with a
  negative `SemiMajorAxis` and times them by `MeanMotion` (deg/day); the port rejected the
  negative axis, so the body had no orbit. `orbit.rs` now takes |a| and MeanMotion.
