# 11 Rendering of bodies and deep sky

## Paths in scope
- crates/se_engine/src/atmosphere.rs, crates/se_core/src/atm.rs (data only), rings.rs, rings3d.rs,
  aurora.rs, eclipse.rs, corona.rs, comets.rs, brown_dwarf.rs, skin.rs
- crates/se_engine/src/nebulae.rs, nebula_raymarch.rs, galaxies.rs (1.6k lines), ellipticals.rs,
  blackholes.rs, blackhole_params.rs, grids.rs, constellations.rs
- shaders for the above (atmosphere_common, clouds, rings*, aurora, eclipse, corona, comet,
  brown_dwarf, star_surface, sky, galaxy_*, blackhole_lens, accretion_disk) where uniforms/buffers
  are set up or precision/NaN matters

## What it does
Draws atmospheres (scattering LUTs), clouds, planetary rings (flat and 3D), auroras, eclipse shadows,
star coronas and surfaces, comets' tails, brown dwarfs, nebulae (volumetric raymarch), galaxies near
and far, black holes with lensing, and the coordinate grids and constellation lines.

## Likely bug classes
- CPU/GPU desync: uniform struct layout/padding not matching the WGSL struct, a uniform written for
  the wrong entity, values one frame late, a buffer reused between two bodies.
- Leaks: materials/meshes/textures/LUTs created per frame or per visit and never freed (visiting many
  planets or systems grows memory), entities not despawned when the system changes.
- Wrong results: eclipse shadow from the wrong light or wrong body, ring shadow orientation, units
  (km vs m vs AU) in uniforms, camera-relative origin missing → jitter or wrong place at distance.
- NaN/inf: zero-radius rings/atmospheres, camera exactly at a body centre, normalise of zero vectors,
  log/pow of zero in shaders that blank the whole frame.
- Panics: empty meshes, zero-size textures.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. Port runs: `select NAME`, `goto NAME DIST_KM`, `shot FILE`.
