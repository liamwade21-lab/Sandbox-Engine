"""Writes .audit/tickets/*.md from the table below. Usage: python -I .audit/tools/make_tickets.py"""
import os

HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "tickets")

T = {
"01-stars": ("Stars (rendering and catalog stars)",
"""Catalog and procedural stars as seen from afar and close up: point/sprite drawing (`star_point.glsl`,
`star_sprite.glsl`, `star_sprite_blur.glsl`, `billboard.glsl`), the magnitude -> brightness/size model,
glare and diffraction, star colours from temperature/class, magnitude limit and auto-brightness
(main-def.cfg `PointRendering` 152-184 and the star keys of `Interface`), how a star looks when you are
next to it (surface `tg_sun_color/height/glow.glsl`, granulation, spots, limb darkening, prominences),
coronae (`corona_plane_proc.glsl`, `corona_plane_tex.glsl`), lens flares (`lens_flare.glsl`, `textures/flares`),
white dwarfs, neutron stars, pulsars and jets (`accretion_jet.glsl`), black holes and lensing (`einstein.glsl`,
`gr_geodesic.glh`, `gr_intersection.glh`), brown dwarfs as stars, multiple-star systems (barycentres,
component names like "X Aa/Ab", orbits of the components), star catalogs (`Catalogs.pak` stars: every key
SE reads for a star, e.g. Teff, Lum, Radius, Class, MassSol, variability), star lighting of planets (colour,
luminosity), star selection/picking and how a star's disc/point is kept visible at every distance.""",
"""The main session is root-causing one known bug itself: "the star Regor Ab (gamma Velorum), selected from
0.25 AU away, is invisible". Do not spend time on that one; DO report any other star-visibility problem.
Out of this ticket: star clusters (02), procedural star physical parameters and the systems around them (06),
star map/universe map views (08), global HDR/bloom/tone mapping (09).""",
"`crates/se_engine/src/{stars,procstars,corona,lensflare,blackholes,system}.rs`, `crates/se_core/src/{star,star_octree,catalog,lens_flare}.rs`, shaders `stars.wesl`, `star_surface.wesl`, `corona.wesl`, `lens_flare.wesl`, `pseudo_flare.wesl`, `blackhole_lens.wesl`, `accretion_disk.wesl`, docs `blackholes.md`, `formats/lens_flares.md`"),
"02-clusters": ("Star clusters",
"""Catalog clusters (`Catalogs.pak` cluster files and every key SE reads for them) and procedural `RC`
clusters (main-def.cfg `ClusterGenerator` 978-1015): how a cluster is drawn far away (sprite, point cloud),
how it turns into individual stars as you approach (the member stars become real, selectable stars with
their own names and planetary systems), open vs globular vs embedded/nucleus, member-star generation
(count, mass function, radial profile, King/Plummer model, colours, ages), cluster brightness and its
fade/LOD thresholds, selection and info of clusters and their members, clusters in other galaxies.""",
"""The main session is root-causing one known bug itself: "star cluster sprites (near Pozzo 1) fade out as
you fly up to them, instead of turning into real stars as in SE". Do not root-cause that one, but DO find
and document SE's mechanism for the far-sprite -> member-stars transition (functions, thresholds, how member
stars are generated and named) and list each part of it the port lacks: that evidence is valuable. Also
write what you learn about that mechanism (function addresses, thresholds, member-star generation) to
`.audit/work/02/cluster_mechanism.md`.
Out of this ticket: field stars and star rendering in general (01), nebulae (03).""",
"`crates/se_engine/src/clusters.rs`, `crates/se_engine/src/procstars.rs`, `crates/se_core/src/{proc_galsys,catalog,star_octree}.rs`, docs `formats/procedural_deepsky.md`"),
"03-galaxies-nebulae": ("Galaxies and nebulae",
"""Galaxies: sprite/particle models (`galaxy_model.glsl`, `galaxy_bulge.glsl`, `galaxy_plane.glsl`,
`billboard.glsl`, `particle.glsl`), types (spiral, barred, elliptical, lenticular, irregular), dust, the view
from inside a galaxy (Milky Way background), far galaxy points and `AutoGalaxyMagnitude`, procedural galaxies
(main-def.cfg `GalaxyGenerator` 1048-1059, `RG` names), quasars/AGN if SE has them, galaxy catalogs
(`Catalogs.pak` galaxy keys). Nebulae: models (`nebula_model.glsl`, `models/nebulae`), ray-marched nebulae
(`nebula_raymarch.glsl`, `nebula_raymarch_uniforms.glh/.cfg`, `raymarch_common.glh`, `models/custom`), dark
nebulae, planetary nebulae, supernova remnants, procedural `RN` nebulae (`NebulaGenerator` 1015-1048),
nebula catalogs. Brightness, colour and LOD behaviour of all of these when approached and flown through.""",
"""Out of this ticket: star clusters (02), the galaxy map / universe map views (08). Redshift is in (it is a
galaxy colour effect).""",
"`crates/se_engine/src/{galaxies,procgalaxies,ellipticals,nebulae,nebula_raymarch}.rs`, `crates/se_core/src/{galaxy_model,proc_galaxy,proc_galsys,catalog}.rs`, shaders `galaxy_*.wesl`, docs `formats/galaxy_model.md`, `formats/nebula*.md`, `formats/procedural_deepsky.md`"),
"04-planet-surfaces": ("Planet surfaces, terrain and water",
"""Everything about how a solid or gaseous body's surface is drawn: `planet.glsl`, the `tg_*` generators
(`tg_terra_*`, `tg_selena_*`, `tg_asteroid_*`, `tg_gasgiant_*`, `tg_browndwarf_*`, `tg_common.glh`,
`tg_detail_maps.glsl`, `terrain_noise.glh`, `terrain_pbr.glh`, `texture_mix.glh`, `bump_to_norm*.glsl`),
`water.glsl`, night lights and lava/volcanic glow (`*_glow.glsl`), gas giants' bands, storms and colours,
terrain LOD and quality (main-def.cfg `PlanetRendering` 85-123, `TextureRendering` 297-351, the planet parts
of `Lighting` 240-297), textured planets' tile packs, asteroid and small-body models, ice, craters, rivers,
volcanoes if present, surface physics like ground collision height, what planets look like from orbit, from
the ground and at night. Catalog planet keys that affect appearance (Surface, Ocean, Lava, NightLights...).""",
"""Out of this ticket: atmospheres, clouds, rings, eclipses, comets and auroras (05); how procedural planets'
parameters are generated (06); global post-processing (09).""",
"`crates/se_engine/src/{terrain,terrain_gen,surface_tex,system,body_models}.rs`, `crates/se_core/src/{surface_gen,proc_surface,planet_tiles,biomes,tg,body_params,noise}.rs`, shaders `planet.wesl`, `tg_height.wesl`, `model.wesl`, docs `procgen.md`, `formats/surface_params.md`, `formats/biome_presets.md`, `formats/planet_tiles.md`, `formats/se_shaders.md`"),
"05-atmospheres-rings": ("Atmospheres, clouds, rings, eclipses, comets, auroras",
"""`sky.glsl`, `atmo_common.glh`, `atmo_transm.glsl`, SE's runtime atmosphere-table generation `ag_*.glsl`
(does SE compute tables for procedural atmospheres? the port reads shipped `.atm` files), clouds
(`tg_clouds_color/height/glow.glsl`: layers, volumetric or not, cloud shadows, storms), rings (`rings.glsl`,
`rings_raymarch.glsl`, `rings_common.glh`: lighting, shadows, particles close up), eclipses and shadows
(`eclipse_common.glh`), comets (`comet_tail.glsl`), auroras (`aurora.glsl`), haze, twilight, sunsets,
the sky colour from the ground and the limb from space, planetshine, main-def.cfg `AtmosphereRendering`
123-138, `RingsRendering` 138-152 and the shadow parts of `Lighting` 240-297. Catalog keys for
Atmosphere, Clouds, Rings, Comet, Aurora blocks.""",
"""Out of this ticket: the ground and water surface (04); climate/composition generation (06).""",
"`crates/se_engine/src/{atmosphere,rings,comets,aurora,system}.rs`, `crates/se_core/src/{atm,rings,body_params}.rs`, shaders `sky.wesl`, `atmosphere_common.wesl`, `clouds.wesl`, `rings.wesl`, `eclipse.wesl`, `comet.wesl`, `aurora.wesl`, docs `atmosphere.md`, `formats/rings.md`, `formats/climate.md`"),
"06-procedural-generation": ("Procedural generation of stars, systems and bodies",
"""What SE generates and the data it shows for it: procedural stars' physical parameters (main-def.cfg
`StarGenerator` 966-978: class/mass distribution, multiplicity, ages, remnants), planetary systems
(`SolarSystemGenerator` 863-896: planet count, orbits, resonances, belts, Trojans, rogue planets, moons,
rings, comets, asteroids), bodies (`PlanetGenerator` 933-966: class, mass, radius, composition, temperature,
atmosphere, oceans, surface params), life (`LifeGenerator` 896-933), naming of procedural objects, and the
derived physical quantities SE's info panel shows for each object (look at `eng-gui.cfg` panel labels and the
exe code that fills them). Compare outcomes too: the port's se_core has decoded large parts (see docs); find
what is still approximated, missing or different. Running the port on a few procedural systems helps.""",
"""Out of this ticket: how stars/planets are rendered (01, 04, 05); procedural galaxies/clusters/nebulae
placement (02, 03); the UI panels themselves (08) except for which quantities they can show.""",
"`crates/se_core/src/{procgen/mod.rs,system_gen,surface_gen,climate,climate/*,star,star_octree,body_params,orbit}.rs`, `crates/se_engine/src/{procsystems,procstars,star_search}.rs`, docs `procgen.md`, `formats/system_gen.md`, `formats/climate.md`, `formats/surface_params.md`, `formats/panel_temperature.md`"),
"07-camera-time": ("Camera, navigation, controls and time",
"""Camera modes (free, follow, sync rotation / surface lock, orbit, track/look at, chase), Goto/Center/
Follow/Land behaviour and timing, automatic speed and acceleration/inertia, velocity presets
(main-def.cfg `Controls` 793-823, `VelocityPresets` 823-854, `Physics` 439-453), ground collision, every
keyboard/mouse/gamepad binding SE has (controller cfgs `config/def_generic_controller.cfg`,
`def_xbox360_controller.cfg` list SE's action names; keybinding strings in the exe and `eng-gui.cfg`),
mouse sensitivity/smoothing/inversion, FOV, roll, the time system (time rate steps, reverse, pause, real time,
the date/time dialog, light-time options), reference frames and coordinate systems, `Ephemerides` 854-863,
saved places/locations behaviour (go to place, save place), console commands for navigation.""",
"""Out of this ticket: spacecraft flight (10); camera projections and stereo (09); UI windows other than the
navigation/time controls themselves (08).""",
"`crates/se_engine/src/{camera,universe,hud,scripting}.rs`, `crates/se_core/src/{time,coords,upos,places,ephemeris,jpl_de,orbit,rotation}.rs`, README Controls"),
"08-ui-hud": ("User interface, HUD, maps and overlays",
"""Every SE window, panel, menu and on-screen overlay (from `eng-gui.cfg` sections and the exe): main menu
(`textures/common/Menu.pak`, splash screens, `banner.cfg`), toolbars (`Toolbars` 615-625), info/selection
panel and its fields, wiki, search/find, star browser, solar system browser (`SolarSystemBrowser` 625-636),
solar system chart (`SolarSystemChart` 636-651), universe map / star map / galaxy map modes
(`UniverseMap` 651-664, `star_map_*.glsl`, `galaxy_map_*.glsl`, `map_sphere.glsl`), labels, markers
(`body_mark.glsl`), orbits (`orbit*.glsl`), grids, constellations/asterisms if any, velocity/direction
indicators, object filters and overlay filters (`ObjectFilters` 664-688, `OverlayFilters` 688-754), debug
overlay (`DebugOverlay` 754-793), fps meter, `Interface` 525-578 and `Units` 578-615 keys, skins
(`Skins.pak`, `gui.glsl`), fonts, localization (27 languages), tooltips/hints, places window, console,
screenshot/notification messages.""",
"""Out of this ticket: graphics/settings window contents (09), camera/time controls behaviour (07),
spacecraft HUD (10). Report missing windows and missing fields/options inside existing ones.""",
"`crates/se_engine/src/{hud,overlay,browser,wiki,i18n,settings,star_search}.rs`, `crates/se_core/src/{locale,places}.rs`, docs `formats/config_locale.md`"),
"09-graphics-settings": ("Settings, graphics and post-processing",
"""The settings windows and what each graphics option does: main-def.cfg `Display` 47-62,
`RenderingQuality` 62-76, `DepthBuffer` 76-85, `PostEffects` 196-240, global parts of `Lighting` 240-297,
`TextureRendering` (global options), `Projection` 351-387, `Export` 453-475 (screenshots, video via
`config/ffmpeg.cfg`), `Loader` 475-501, `Memory` 6-26, `Log` 501-525, `config/preset-*.cfg`; shaders
`hdr_*.glsl` (HDR pipeline, auto exposure with `hdr_histogram.glsl`, bloom/glow `hdr_gauss_blur.glsl`,
tone mapping operators `hdrt_*_tone.glh`), `fxaa.glsl`, `upscale_*.glsl`, `combine_temporal.glsl`,
`bilateral.glsl`, `proj_cylinder.glsl`, `proj_fisheye.glsl`, `3D_anaglyph.glsl`, `3D_interlace.glsl`
(stereo, not VR), `pbr_*_cubemap.glsl`, `compress.glsl`. Settings persistence, photo mode, screenshot
formats and resolution multipliers, the cubemap/panorama export.""",
"""Out of this ticket: star brightness settings (01), planet/atmosphere quality knobs' visual result (04/05),
UI windows other than settings (08).""",
"`crates/se_engine/src/{graphics,settings,config,main,camera,lensflare}.rs`, `config/preset-*.cfg` handling in `graphics.rs`"),
"10-scripts-audio-spacecraft": ("Scripts, audio and spacecraft",
"""Scripts: every command and parameter of SE's `.se` script language (command table in the exe, all scripts
and tutorials in `data/scripts/Scripts.pak`), triggers, variables, tutorials' GUI hooks, the console.
Audio: `Sound` 424-439 keys, `data/sounds/menu`, `data/sounds/spacecraft` (engine, warp, UI sounds), music
context switching (`data/music/context.cfg`: which classes/situations pick which tracks, fades, volume).
Spacecraft: `models/spacecraft/ShipsModels.pak`, `textures/spacecraft`, `ship.glsl`, `ship_pbr.glsl`,
`ship_engines.glsl`, `ship_*_engine.glh` (engine plume types), warp/hyperdrive visuals (`WarpRendering`
184-196), flight model, autopilot, docking, landing gear, lights, HUD of the ship, aerodynamics, fuel,
damage, multiple ships, ship selection UI.""",
"""Out of this ticket: the ship editor (out of scope), free-camera navigation (07).""",
"`crates/se_engine/src/{scripting,music,ship,ship_assets}.rs`, `crates/se_core/src/{se_script,ship,model_script,dds}.rs`, shaders `ship.wesl`, `ship_plume.wesl`, docs `formats/se_script.md`, `formats/spacecraft.md`"),
}

for k, (title, scope, notes, port) in T.items():
    tid = k[:2]
    body = f"""# Ticket {tid}: {title}

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `{tid}`; write `.audit/gaps/{tid}.txt`; scratch files go in `.audit/work/{tid}/`.

## Scope
{scope}

## Notes and boundaries
{notes}

## Port starting points
{port}

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
"""
    with open(os.path.join(HERE, k + ".md"), "w", encoding="utf-8") as fh:
        fh.write(body)
print(sorted(T))
