# Ticket 03: Galaxies and nebulae

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `03`; write `.audit/gaps/03.txt`; scratch files go in `.audit/work/03/`.

## Scope
Galaxies: sprite/particle models (`galaxy_model.glsl`, `galaxy_bulge.glsl`, `galaxy_plane.glsl`,
`billboard.glsl`, `particle.glsl`), types (spiral, barred, elliptical, lenticular, irregular), dust, the view
from inside a galaxy (Milky Way background), far galaxy points and `AutoGalaxyMagnitude`, procedural galaxies
(main-def.cfg `GalaxyGenerator` 1048-1059, `RG` names), quasars/AGN if SE has them, galaxy catalogs
(`Catalogs.pak` galaxy keys). Nebulae: models (`nebula_model.glsl`, `models/nebulae`), ray-marched nebulae
(`nebula_raymarch.glsl`, `nebula_raymarch_uniforms.glh/.cfg`, `raymarch_common.glh`, `models/custom`), dark
nebulae, planetary nebulae, supernova remnants, procedural `RN` nebulae (`NebulaGenerator` 1015-1048),
nebula catalogs. Brightness, colour and LOD behaviour of all of these when approached and flown through.

## Notes and boundaries
Out of this ticket: star clusters (02), the galaxy map / universe map views (08). Redshift is in (it is a
galaxy colour effect).

## Port starting points
`crates/se_engine/src/{galaxies,procgalaxies,ellipticals,nebulae,nebula_raymarch}.rs`, `crates/se_core/src/{galaxy_model,proc_galaxy,proc_galsys,catalog}.rs`, shaders `galaxy_*.wesl`, docs `formats/galaxy_model.md`, `formats/nebula*.md`, `formats/procedural_deepsky.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
