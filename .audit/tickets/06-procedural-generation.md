# Ticket 06: Procedural generation of stars, systems and bodies

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `06`; write `.audit/gaps/06.txt`; scratch files go in `.audit/work/06/`.

## Scope
What SE generates and the data it shows for it: procedural stars' physical parameters (main-def.cfg
`StarGenerator` 966-978: class/mass distribution, multiplicity, ages, remnants), planetary systems
(`SolarSystemGenerator` 863-896: planet count, orbits, resonances, belts, Trojans, rogue planets, moons,
rings, comets, asteroids), bodies (`PlanetGenerator` 933-966: class, mass, radius, composition, temperature,
atmosphere, oceans, surface params), life (`LifeGenerator` 896-933), naming of procedural objects, and the
derived physical quantities SE's info panel shows for each object (look at `eng-gui.cfg` panel labels and the
exe code that fills them). Compare outcomes too: the port's se_core has decoded large parts (see docs); find
what is still approximated, missing or different. Running the port on a few procedural systems helps.

## Notes and boundaries
Out of this ticket: how stars/planets are rendered (01, 04, 05); procedural galaxies/clusters/nebulae
placement (02, 03); the UI panels themselves (08) except for which quantities they can show.

## Port starting points
`crates/se_core/src/{procgen/mod.rs,system_gen,surface_gen,climate,climate/*,star,star_octree,body_params,orbit}.rs`, `crates/se_engine/src/{procsystems,procstars,star_search}.rs`, docs `procgen.md`, `formats/system_gen.md`, `formats/climate.md`, `formats/surface_params.md`, `formats/panel_temperature.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
