# 05 Procedural star systems (stars, planets, moons, rings, comets, life)

## Paths in scope
- crates/se_core/src/procgen/mod.rs (3.2k lines) and procgen/se_*.rs (layout, moons, multiple stars,
  rings, comets, life, stars, smbh)
- crates/se_core/src/system_gen.rs, cluster_stars.rs
- crates/se_engine/src/procsystems.rs

## What it does
Generates SE's procedural star systems from a seed: multiple-star hierarchies, planets and moons
with orbits and physical parameters, rings, asteroid belts, comets, life. Must be deterministic: the
same star always gives the same system (names like "RS 8514-1828-5-27938-309 2" depend on it).

## Likely bug classes
- Non-determinism: HashMap iteration order, RNG consumed in a different order on different paths,
  float results depending on evaluation order, state carried between generations.
- Panics: empty ranges in RNG (`gen_range(a..a)`), index out of bounds when a system has 0 planets
  or many stars, unwrap on missing parent, recursion depth.
- Wrong results: orbits that overlap/cross, moons outside the Hill sphere or inside the planet,
  planets inside the star, wrong parent/barycentre links, counts including the wrong kind of object.
- Names/IDs: the same body getting two names, or two bodies the same name (selection and bookmarks
  rely on these).
- Engine side (procsystems.rs): caches of generated systems that grow forever or get evicted while in use.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. crates/se_core/examples/procgen_dbg.rs shows how to generate a system;
scratch crates can generate many seeds quickly to look for panics/NaN.
