# Bugs found while porting (not in the audit)

- **Name lookup ignores case**: `Catalog::find` keys names lowercased, so the planet "Kepler-34 b"
  resolves to the star "Kepler-34 B" (same for Kepler-16/35/47 b). SE keeps them apart.
  Found 2026-10-30 while testing lighting by several stars. `crates/se_core/src/catalog.rs:981`.
- ~~**'Oumuamua sits at the Sun's centre**~~ Fixed 2026-10-30: SE writes hyperbolic orbits with a
  negative `SemiMajorAxis` and times them by `MeanMotion` (deg/day); the port rejected the
  negative axis, so the body had no orbit. `orbit.rs` now takes |a| and MeanMotion.
