# 02 Procedural planet surfaces, climate and biomes (CPU side)

## Paths in scope
- crates/se_core/src/surface_gen.rs, proc_surface.rs, biomes.rs, noise.rs
- crates/se_core/src/climate.rs, climate/display.rs (climate/tests.rs for reference)
- crates/se_core/src/atm_gen.rs

## What it does
Turns a body's parameters (mass, temperature, life, ocean, etc.) into SE's procedural surface
parameters (the values fed to tg_height.wesl / planet.wesl), climate zones, biome colours and
generated atmospheres. These must match SE's own generator, so wrong results here mean planets
look/behave differently from SE.

## Likely bug classes
- Wrong results: formula transcribed from SE wrong (sign, units K vs C, degrees vs radians, km vs m),
  wrong branch for a planet class, clamping missing so values go out of range.
- Division by zero / NaN for edge cases: zero mass, zero radius, no atmosphere, temperature 0,
  tidally locked, gas giants with no surface, an empty biome list.
- Panics: indexing tables with out-of-range values from data, unwrap on optional fields.
- Determinism: same body generating differently on different runs (HashMap order, uninitialised
  seeds, float order), which would make planets change between visits.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. `cargo test -p se_core` runs the existing tests; scratch repro crates
against se_core are cheap. The examples in crates/se_core/examples show how to load bodies.
