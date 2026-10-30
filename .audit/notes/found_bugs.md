# Bugs found while porting (not in the audit)

- **Name lookup ignores case**: `Catalog::find` keys names lowercased, so the planet "Kepler-34 b"
  resolves to the star "Kepler-34 B" (same for Kepler-16/35/47 b). SE keeps them apart.
  Found 2026-10-30 while testing lighting by several stars. `crates/se_core/src/catalog.rs:981`.
- **'Oumuamua sits at the Sun's centre**: its hyperbolic orbit is not placed, so the body is at
  its parent's position (it reflected 1e22 x sunlight into planet shine until bodies inside their
  star were excluded). Probably every hyperbolic catalog orbit. Found 2026-10-30.
