# Ticket 04: Planet surfaces, terrain and water

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `04`; write `.audit/gaps/04.txt`; scratch files go in `.audit/work/04/`.

## Scope
Everything about how a solid or gaseous body's surface is drawn: `planet.glsl`, the `tg_*` generators
(`tg_terra_*`, `tg_selena_*`, `tg_asteroid_*`, `tg_gasgiant_*`, `tg_browndwarf_*`, `tg_common.glh`,
`tg_detail_maps.glsl`, `terrain_noise.glh`, `terrain_pbr.glh`, `texture_mix.glh`, `bump_to_norm*.glsl`),
`water.glsl`, night lights and lava/volcanic glow (`*_glow.glsl`), gas giants' bands, storms and colours,
terrain LOD and quality (main-def.cfg `PlanetRendering` 85-123, `TextureRendering` 297-351, the planet parts
of `Lighting` 240-297), textured planets' tile packs, asteroid and small-body models, ice, craters, rivers,
volcanoes if present, surface physics like ground collision height, what planets look like from orbit, from
the ground and at night. Catalog planet keys that affect appearance (Surface, Ocean, Lava, NightLights...).

## Notes and boundaries
Out of this ticket: atmospheres, clouds, rings, eclipses, comets and auroras (05); how procedural planets'
parameters are generated (06); global post-processing (09).

## Port starting points
`crates/se_engine/src/{terrain,terrain_gen,surface_tex,system,body_models}.rs`, `crates/se_core/src/{surface_gen,proc_surface,planet_tiles,biomes,tg,body_params,noise}.rs`, shaders `planet.wesl`, `tg_height.wesl`, `model.wesl`, docs `procgen.md`, `formats/surface_params.md`, `formats/biome_presets.md`, `formats/planet_tiles.md`, `formats/se_shaders.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
