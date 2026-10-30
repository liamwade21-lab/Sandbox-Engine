# Ticket 01: Stars (rendering and catalog stars)

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `01`; write `.audit/gaps/01.txt`; scratch files go in `.audit/work/01/`.

## Scope
Catalog and procedural stars as seen from afar and close up: point/sprite drawing (`star_point.glsl`,
`star_sprite.glsl`, `star_sprite_blur.glsl`, `billboard.glsl`), the magnitude -> brightness/size model,
glare and diffraction, star colours from temperature/class, magnitude limit and auto-brightness
(main-def.cfg `PointRendering` 152-184 and the star keys of `Interface`), how a star looks when you are
next to it (surface `tg_sun_color/height/glow.glsl`, granulation, spots, limb darkening, prominences),
coronae (`corona_plane_proc.glsl`, `corona_plane_tex.glsl`), lens flares (`lens_flare.glsl`, `textures/flares`),
white dwarfs, neutron stars, pulsars and jets (`accretion_jet.glsl`), black holes and lensing (`einstein.glsl`,
`gr_geodesic.glh`, `gr_intersection.glh`), brown dwarfs as stars, multiple-star systems (barycentres,
component names like "X Aa/Ab", orbits of the components), star catalogs (`Catalogs.pak` stars: every key
SE reads for a star, e.g. Teff, Lum, Radius, Class, MassSol, variability), star lighting of planets (colour,
luminosity), star selection/picking and how a star's disc/point is kept visible at every distance.

## Notes and boundaries
The main session is root-causing one known bug itself: "the star Regor Ab (gamma Velorum), selected from
0.25 AU away, is invisible". Do not spend time on that one; DO report any other star-visibility problem.
Out of this ticket: star clusters (02), procedural star physical parameters and the systems around them (06),
star map/universe map views (08), global HDR/bloom/tone mapping (09).

## Port starting points
`crates/se_engine/src/{stars,procstars,corona,lensflare,blackholes,system}.rs`, `crates/se_core/src/{star,star_octree,catalog,lens_flare}.rs`, shaders `stars.wesl`, `star_surface.wesl`, `corona.wesl`, `lens_flare.wesl`, `pseudo_flare.wesl`, `blackhole_lens.wesl`, `accretion_disk.wesl`, docs `blackholes.md`, `formats/lens_flares.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
