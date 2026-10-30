# Ticket 05: Atmospheres, clouds, rings, eclipses, comets, auroras

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `05`; write `.audit/gaps/05.txt`; scratch files go in `.audit/work/05/`.

## Scope
`sky.glsl`, `atmo_common.glh`, `atmo_transm.glsl`, SE's runtime atmosphere-table generation `ag_*.glsl`
(does SE compute tables for procedural atmospheres? the port reads shipped `.atm` files), clouds
(`tg_clouds_color/height/glow.glsl`: layers, volumetric or not, cloud shadows, storms), rings (`rings.glsl`,
`rings_raymarch.glsl`, `rings_common.glh`: lighting, shadows, particles close up), eclipses and shadows
(`eclipse_common.glh`), comets (`comet_tail.glsl`), auroras (`aurora.glsl`), haze, twilight, sunsets,
the sky colour from the ground and the limb from space, planetshine, main-def.cfg `AtmosphereRendering`
123-138, `RingsRendering` 138-152 and the shadow parts of `Lighting` 240-297. Catalog keys for
Atmosphere, Clouds, Rings, Comet, Aurora blocks.

## Notes and boundaries
Out of this ticket: the ground and water surface (04); climate/composition generation (06).

## Port starting points
`crates/se_engine/src/{atmosphere,rings,comets,aurora,system}.rs`, `crates/se_core/src/{atm,rings,body_params}.rs`, shaders `sky.wesl`, `atmosphere_common.wesl`, `clouds.wesl`, `rings.wesl`, `eclipse.wesl`, `comet.wesl`, `aurora.wesl`, docs `atmosphere.md`, `formats/rings.md`, `formats/climate.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
