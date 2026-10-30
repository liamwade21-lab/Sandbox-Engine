# SpaceEngine — Rust/Bevy port

An internal reimplementation of SpaceEngine 0.991 in Rust on the Bevy 0.20 engine,
rendering through **Vulkan**. It loads the original game data (catalogs, planet
textures, atmosphere tables, models, music, saved places, the JPL ephemeris) directly
from an existing SpaceEngine installation at runtime. No SpaceEngine assets are
copied into this repository.

> Internal use only. The encrypted `data/shaders/Shaders.pak` is not read; all
> shaders here are new WESL code.

## Requirements

- Rust 1.97 or newer (stable).
- A Vulkan-capable GPU and driver. The renderer is forced to the Vulkan backend.
- A SpaceEngine 0.991 installation. The default path is
  `D:/SteamLibrary/steamapps/common/SpaceEngine`.

## Build and run

```bash
cargo run --release -p se_engine
```

```bash
cargo run --release -p se_engine -- --se-path "D:/SteamLibrary/steamapps/common/SpaceEngine" --start Mars
```

| Option | Meaning |
|---|---|
| `--se-path DIR` (or env `SE_PATH`) | SpaceEngine install directory |
| `--start NAME` | object to start next to (default Earth) |
| `--size WxH` | window size |
| `--run "cmd; cmd; ..."` | scripted commands (see below), useful for demos and tests |
| `--script PATH.se` | run a SpaceEngine script: a VFS path such as `scripts/menu/Scene1.se` or `tutorials/tut-01-basic-mov.se`, or a file on disk (see "SpaceEngine scripts") |
| `--script-auto` | with `--script`: let waits for the user (Ok buttons, triggers) end by themselves after a few seconds, for unattended runs |
| `--screenshot FILE.png --frames N` | save a screenshot after the script plus N frames, then exit |
| env `SE_NO_MUSIC=1` | start with music disabled |
| env `SE_NO_BLOOM=1` | disable bloom |
| env `SE_NO_BLACKHOLES=1` | disable the black-hole lensing pass |
| env `SE_NO_PROCGAL=1` | disable procedural galaxies, clusters and nebulae; `SE_PROC=g`, `c`, `n` (or a mix) enables only those |
| env `SE_NO_VSYNC=1` | uncap the frame rate, for performance measurements |
| env `SE_SHOW_FPS=1` | show the frame rate in the camera toolbar (off by default, as in SE; the filters toolbar's "FPS" saves it) |
| env `SE_SKIN=NAME`, `SE_UI_OPEN=settings,graphics,...`, `SE_OVERLAY=orbits,labels=2,...` | testing: start with another interface skin; open windows (settings, graphics, find, places, wiki, help) at start; switch overlays on (`orbits`, `labels`, `labels=2`, `markers`, or SE's filter names such as `ShowLabelsGalaxies`) |
| env `SE_TONEMAP=aces` | the earlier ACES tone curve instead of SpaceEngine's default AgX (0.75 exposure, saturation 1.5 as a vibrance) |
| env `SE_FRAMETIME_LOG=FILE` | write one line per frame (`frame ms nearest-star`) to FILE, to find hitches |

Script commands: `goto NAME [km]`, `select NAME`, `look NAME`, `surface LAT LON ALT_KM [PITCH] [HEADING]`,
`child N [km]` (Nth planet of the current system), `nearest`, `place TEXT` (a saved SpaceEngine place),
`at X Y Z` (parsecs from the Sun), `gal L B DIST_PC`, `galactic DIST_PC`, `date YYYY-MM-DD [HH:MM:SS]`,
`rate X`, `pause`, `rs RS L-X-Y-Z-I` (jump next to a procedural star by its name), `search LY [life|bh]` (the Star browser's search, for life or black holes; `SE_BROWSER_OPEN=search` shows it), `shot FILE` (save a screenshot and carry on), `speed KMS` (assigned velocity, km/s), `fly KMS` (dev: keep flying forward at KMS km/s, fixed step per frame), `click X Y` (dev: a left click at window pixel X, Y), `go [KM]` (Go to the selection at once, setting the velocity like G; script `goto` keeps it), `fov DEG`, `move X Y Z`, `turn YAW PITCH`, `shore [M]` (dev: move to the nearest coast, M metres inland, facing the sea), `agl M` (dev: hold the camera M metres above the ground under it, `agl off` ends it), `wait FRAMES`.
Spacecraft commands: `ship NAME` (spawn a design by id or name, e.g. `ship DragonFly`, and enter flight mode),
`throttle X` (-1 to 1), `warp LOG10` (engage the warp drive at that boost, `warp off`), `flight on|off`, `unship`,
`shipcam N` (chase preset N, past the last one is the orbit view), `shiporbit YAW PITCH [DIST_M]`,
`shiprcs X Y Z` and `shipturn PITCH YAW ROLL` (held commands, -1 to 1). Example:
`--run "goto Earth 8000; ship DragonFly; throttle 0.5; wait 200"`. Env `SE_SHIPS_WINDOW=1` opens the Ships window at start.

SpaceEngine scripts (`.se`, see `docs/formats/se_script.md`) run with `--script`, or from the **Scripts**
window, which lists the ones shipped in `data/scripts` (menu scene, tutorials). Supported: `GotoLocation`,
`Goto { Height/DistKm/DistRad Lat Lon Yaw Pitch Roll Time Center }`, `Select`/`Unselect`, `Center`, `Follow`,
`SyncRot`, `Free`, `Orbit`/`StopOrbit`, `FOV`, `SpeedKm`, `TimeScale`, `Date`, `StopTime`/`StartTime`, `Wait`,
`WaitTrigger` (+ `BeginMultiTrigger`/`EndMultiTrigger`) for the camera, selection, time and distance events,
`ShowMessage`/`WaitMessage`/`Print` (shown as an on-screen caption), `Set`/`SetForce`/`WaitVar`/`SaveVars`/`RestoreVars`/
`Interpolate`, `If`/`Else`/`Loop`, `Run`. Tutorial widget commands (`HighlightWidget`, `ShowDialog`, `UserMoveControl`, ...),
waypoints and sounds are logged once and skipped, and triggers on GUI actions the port lacks time out after 4 s.
Scripted runs (`--script`) do not read or write the saved settings. Env `SE_SCRIPTS_WINDOW=1` opens the Scripts window at start.

Tests (engine-independent core, about 190 tests):

```bash
cargo test -p se_core
```

## Controls

| Input | Action |
|---|---|
| Left drag | look around |
| Right drag | orbit the followed object (after Go to), or the selection |
| Left + right drag | zoom: move towards / away from that object |
| Shift + left drag, Page Up / Page Down | change the field of view (1 arcsecond to 120 degrees, zooms animate over 0.25 s); middle click zooms to 20 degrees and back, Home resets it. Zooming works like a telescope: stars, planets and clusters drawn as points get `(FOV / 45°)^-1.5` brighter, so fainter ones show (loaded inside the view down to magnitude 20) |
| `]` / `[`, Ctrl+`]` / `[`, P | raise / lower the magnitude limit by 0.1 (all, or stars only), reset it to the startup value |
| Click an object, a label or any visible star or galaxy | select it (even a galaxy that is only a dot); a procedural star (`RS ...`, or a cluster member) becomes a real star with planets, so Go to works |
| W/S, A/D, R/F (arrows, Numpad 1/0) | move forward/back, left/right, up/down at the assigned velocity; Z stops |
| Q/E (Numpad 7/9), Numpad 4/6/8/2 | roll, turn (ramped as SE's TurnKeyAccel/Decel); Numpad 5 stops turning; Ctrl+Numpad 5/4/6/8/2 look back/left/right/up/down |
| Shift+D / Shift+F / Shift+R | free / follow / rotate with the selection (SE's binding; also on the navigation bar, which shows "Following" / "Rotating") |
| Shift+G / Ctrl+G / End / T | land on the selection / go to its centre / level the view to its horizon / track it |
| 5 | SE's auto horizon (on by default): bound to a planet and low over it, looking around keeps the horizon level |
| Backspace / Shift+H | previous selection / cycle the home address (Earth, Sun, Solar System, Milky Way) |
| Mouse wheel, + / - | change the assigned velocity (four notches per factor of 10); hitting the ground resets it to 100 m/s; the eye stays 1.5 m above the ground (SE's `ClipHeight`); stars and other bodies without terrain stop the camera just above their surface (two millionths of the radius, 1.4 km at the Sun) |
| Ctrl + wheel, Ctrl + / - | jump to the next / previous velocity preset (0.1 m/s ... 100 Mpc/s, from `config/main-def.cfg`) |
| G | go to the selection and set the assigned velocity to a tenth of the arrival distance per second, as SpaceEngine does, so flying on approaches it instead of rushing past. The flight lasts as SE's (`GotoSpeed` 10: log10 of the distance ratio in seconds, plus one, up to `LongGotoTime` 20 s); G again is the fast Go to (3 s). C centres it in 1.5 s |
| Space | pause time; K/L (and the navigation toolbar's << / >>) step the rate through SE's 1, 3, 10, 30, 100, ... (0.3, 0.1, ... below 1) and resume a paused clock; J reverses; `\` sets real time; Ctrl+`\` the current date; click the date to type one |
| M / N | toggle music / next track (tracks cross-fade, the title shows on screen) |
| F2 / Ctrl+F2 / Alt+F2 | solar system browser at the selection's level / the camera's system / the system's root |
| Alt+Enter / Shift+Enter / Ctrl+Enter | fullscreen / next display / minimize |
| F1 / Ctrl+` | the list of controls (F1 is SE's universe map, not ported) / hide the interface |
| F3 / I | SE's Find object dialog (Enter selects) / the wiki: the selected object's description from SpaceEngine's locale database |
| Shift+F3 / F8 or 0 | star browser (nearest stars, and the search for planets, moons and stars by parameters; also the main toolbar's Search button) / system chart of the current system |
| F4 / Ctrl+F4 / F7 | settings / graphics settings / locations |
| O / ' / Ctrl+' / ; | orbits / labels (brief, detail, off) / markers / selection pointer; each kind of object can be switched in Settings > Filter objects |
| X / Shift+X / Ctrl+X / Alt+X | constellations / their lines / boundaries / names (right click the Constellations button for the switches) |
| Ctrl+; / Alt+; | celestial grids (equatorial by default; ecliptic and galactic on the Celestial grids button's right click) / a planet's geographic grid |
| Shift+. / Shift+, (`>` / `<`) | SE's ambient lighting up / down in steps of 0.05 (none by default), so dark bodies can be seen |
| `.` / `,` / `/` | exposure compensation up / down by a quarter stop (repeats while held; -4 to +42) / reset |

### Flight mode (spacecraft)

Open the **Ships** window (button at the top right, below the bar), pick a design and press **Fly**: the ship appears
where the camera is (its chase camera at the camera's position), at rest relative to the nearest body, and flight mode
starts. Ship data comes from SpaceEngine's `ShipsModels.pak`; see `docs/formats/spacecraft.md`.

| Input | Action |
|---|---|
| Enter (or F outside flight) | toggle flight mode (needs a ship) |
| Esc | leave flight mode; the free camera stays where the chase camera was and the ship keeps drifting |
| W / S | raise / lower the main-engine throttle (S below zero is retro thrust); X cuts it |
| Z / Alt+Z / Shift+Z | stop the engines (and any autopilot program) / stop the warp drive / sync velocity with the reference body |
| A / D, R / F | RCS strafe left/right, up/down (`CorrEngines`, `HoverEngines`) |
| Arrow keys, Q / E | pitch and yaw, roll (`TurnEngines`); Shift is ×10 |
| Mouse drag | turn the ship (chase views) or orbit it (orbit view); wheel zooms the camera |
| U / Numpad 5 | flight assist (rotation damping, on by default) / kill rotation |
| 5 / 6 / 7 / 8 / 9 / 0, End | hold prograde / retrograde / radial / antiradial / normal up / normal down, horizon (relative to the reference body) |
| H | hold altitude |
| T / P | select the camera's selection as the target / cycle the reference body (nearest, strongest gravity, the target) |
| C / Shift+G / Alt+G / G | autopilot: rotate to / fly to / warp to / go to (fly or warp) the target; Z stops it |
| Backspace | brake: match the velocity of the nearest body (the autopilot uses the retro or RCS acceleration) |
| PageUp / PageDown, Alt+`=` / Alt+`-` | warp drive (ships with a `Warpdrive` or `Hyperdrive`): engage and raise the target boost / lower it, stopping at 0 |
| V | cycle the design's `Behind` chase cameras and the free orbit camera |

Engines are accelerations in m/s² (not thrust), warp multiplies velocity by 10^boost with the boost capped as the
ship nears a body, so warp speed grows with altitude. Gravity comes from every body of the system, and the ground is the
terrain (touchdown pins the ship to the rotating planet; 25 m/s or more is a crash notice). The HUD shows ship, speed
relative to the nearest body, throttle, warp state and altitude. The Ships window also toggles gravity and collisions.

The interface follows SE's layout: the selection's info table as text in the upper-left corner
(diameter, magnitudes, luminosity, gravity, density, albedo, temperature, pressure, orbit, rotation,
apparent size, the ground under the camera; brief or full, saved as `TextMode`), and four toolbars:
tools (Find object, **Places** with SpaceEngine's own saved places, **Scripts** for the shipped `.se`
scripts, Search, Wiki, **Settings**, **Graphics** for quality presets, bloom and lens flares, Help)
bottom left, filters (orbits, labels, markers, selection pointer, full info, FPS) on the right,
navigation (date, time rate, Go to, Center, Clear) and camera (nearest body, velocity, FOV, and SE's
"Mag limit": the star limit raised by the zoom, which typing sets back) along the bottom. They stay pinned (SE can auto-hide them) and use text, not SE's icons.
The interface wears SE's skins (Settings, "Interface skin"; default Flat Blue, all 25 of `Skins.pak`): each sets
the windows' and widgets' colours from its texture, its text colours, shadows and capital headers, and windows
show the blurred scene behind them as in SE. The font is SE's NotoSans.
Orbits, labels and markers follow SE's overlay filters (off by default; per kind of object in Settings > Filter
objects) in the skin's colour for each kind: orbits are SE's glowing lines with a trail behind the body, the
selected one red; labels have SE's brief and detail modes and also name galaxies, clusters and nebulae. Key actions
(time, overlays) show SE's short message in the middle of the screen.

## Architecture

```
crates/se_core    engine-independent: formats, catalogs, astronomy, procedural generation
crates/se_engine  Bevy application (binary `spaceengine-rs`)
docs/             reverse-engineering notes and format specifications
re/               Ghidra project and string/RTTI dumps of SpaceEngine.exe
```

### Core (`se_core`)

| Module | Purpose |
|---|---|
| `vfs` | mounts `data/` and `addons/`; reads ZIP `.pak` files (custom central-directory reader, positioned reads, lock-free) |
| `sc` | parser for SpaceEngine's `.sc`/`.cfg` script syntax |
| `catalog` | stars (`.sc` + HIPPARCOS CSV), planets/moons/asteroids/comets, galaxies, clusters, nebulae; name index; hierarchy |
| `coords`, `upos`, `units`, `time` | universe frame, i128 fixed-point positions (1/1024 m), units, Julian dates |
| `orbit`, `rotation`, `ephemeris`, `jpl_de` | Kepler orbits (binary `Separation` / `BinaryOrbit` split by mass, as SE does), IAU/simple rotation, positions at a date, JPL DE436 reader |
| `proc_galaxy`, `proc_galsys` | procedural galaxies (octree, cosmic web, `RG` names) and the clusters/nebulae of a galaxy (`RC`, `RN`) |
| `star`, `star_octree`, `procgen`, `proc_surface`, `surface_gen`, `noise` | stellar physics; procedural stars of every galaxy; procedural planetary systems (emitted as `.sc` text); procedural terrain; SpaceEngine's per-body surface parameter generator and body classes |
| `climate` | SpaceEngine's completion of catalog planets without an `Atmosphere`: bulk composition, tides and heat, gas inventory, loss and escape, climate/phase solve, oceans, atmosphere model choice; gas and XUV tables read from `SpaceEngine.exe` (docs/formats/climate.md) |
| `planet_tiles`, `atm`, `rings`, `body_params`, `model_sm`, `model_script`, `galaxy_model` | data formats: cube-map tiles, scattering tables, rings, sub-blocks, `.sm` meshes, `.sss` scripts, `.pst` galaxy models |
| `ship`, `dds` | spacecraft: `.sss` designs, module `.cfg`, `.sml` materials, flight constants (`docs/formats/spacecraft.md`); DXT texture decoder |
| `locale`, `places` | UI strings and object names (27 languages), saved places |

### Engine (`se_engine`)

| Module | Purpose |
|---|---|
| `universe` | simulation clock; floating origin; logarithmic "scaled space" so 10²⁶ m fits f32 |
| `camera` | free flight, go-to, surface-locked follow, ground clamp, scripted commands |
| `stars`, `procstars`, `procgalaxies`, `procsystems`, `clusters`, `galaxies`, `ellipticals`, `nebulae`, `nebula_raymarch` | catalog and procedural stars, star clusters, galaxy and nebula particle models, volumes of elliptical galaxies, ray-marched nebulae |
| `system`, `terrain`, `surface_tex`, `atmosphere`, `rings`, `body_models`, `comets`, `blackholes` | the star system around the observer: quadtree terrain, biome and soil surface textures, Bruneton atmosphere, clouds, oceans, eclipses, rings, asteroid models, comets, black-hole lensing |
| `ship`, `ship_assets` | spacecraft: Ships window, flight physics (thrust, gravity, warp, ground contact), chase/orbit cameras, hull and plume rendering |
| `hud`, `skin`, `overlay`, `messages`, `music`, `settings`, `graphics`, `lensflare` | egui interface and SE's skins, orbits/labels/markers/picking, splash messages, soundtrack, persistent settings, graphics presets, lens flares |

Key rendering decisions:

- **Positions.** Every object has an `i128` fixed-point universe position; the camera stays at the render origin. Distances beyond a dynamic linear radius are log-compressed (translation and scale together), which preserves angular sizes.
- **Depth.** Reverse-Z depth. Stars and galaxies are drawn at infinity, so planets occlude them naturally.
- **Terrain.** Patches are built on worker threads in f64, relative to each patch centre, which gives millimetre vertex precision anywhere.

## Status

### Implemented

- **Data and catalogs.** VFS over all paks and every catalog: about 135k stars, 9.9k bodies, 11k galaxies, 1.3k clusters, 193 nebulae.
- **Solar System accuracy.** Planets, the Sun, Earth and the Moon come from JPL DE436, as in SpaceEngine. The major moons use SpaceEngine's own analytic theories (L1, TASS17, GUST86, NOE-4, HTC20, Noyelles, Triton), read from the executable at runtime and matching its output to about a metre (`docs/formats/moon_theories.md`). SpaceEngine's saved places reproduce exactly. The Moon turns with SE's `Moon-DE` model (DE436's lunar librations), so its near side faces Earth; `TidalLocked` bodies, and moons with IAU rotation (SE's `IgnoreMoonsRotationIAU`), keep longitude 0 towards their planet. Bodies are SE's ellipsoids: `Oblateness` flattens Saturn, Jupiter, Uranus, Neptune, Earth and Mars at the poles, and `Dimensions` gives Mimas, Enceladus and Haumea three axes.
- **Planet surfaces.** Textured planets use SpaceEngine's tiled cube maps: colour, 16-bit relief, oceans with depth colour and glint, night lights, a separate cloud shell, and cloud shadows. A quadtree LOD with skirts and horizon culling drives them, and procedural detail is added below the data resolution; every level samples the same surface (SpaceEngine's generator has fixed octave counts), so coasts and lakes stay put as patches split; each patch carries a mipmapped, anisotropic normal map at four times its mesh resolution (SE: 256 texels on 33 vertices), so relief finer than the mesh shades from afar, and a freshly split patch starts on its parent's shape and morphs into its own as the camera nears it. Height tiles are read with SE's Catmull-Rom filter, and the normal map's lookups are jittered by up to half a texel as SE's are. Every rocky world is textured like SpaceEngine's: a biome preset (`textures/terrain/samples`, chosen as SpaceEngine does from its body classes, star temperature and generated `SurfStyle`, `docs/formats/biome_presets.md`) gives tints and tiled sample textures per biome (sea floor, shelf, beach, lowland, upland, rock, snow, ice, lava, vegetation), blended by height, slope and climate; close to the ground the preset's soil materials (`textures/terrain/materials`; a layer's frequency is its repeats per 128 m, which puts grass at about a metre per repeat and sand ripples under a metre apart, as SpaceEngine shows them from 1.5 and 20 m) add centimetre-to-metre detail at their full 1024 px, each point taking the soil SpaceEngine's own rule picks (its decrypted `tg_detail_maps` shader: HSV distance of the soil's colour to the colour map plus the slope gap, `docs/formats/se_shaders.md`; Earth's grassland gets grass, the Sahara rippled sand), recoloured to the ground in HSV as SE does, with their normal, roughness and occlusion maps, full within 1 km and gone by 2 km (SE's fade); the sample layers add their own relief (a normal map made from each sample's height map, and its roughness) from about a hundred metres to orbit. Each layer fades in as its repeat grows on screen, so no tiling shows. SE's tiles carry a 2-texel border shared with their neighbours, sampled as such (no cliffs along tile edges), and the LOD measures distance to the ground's real height, so patches under the camera on high ground split fully. Procedural worlds take their colours from the biomes; textured ones keep their maps and gain the detail. Relief and climate of worlds without a bump map follow the surface parameters SpaceEngine generates for each body (mountains, dunes, hills, canyons, craters, maria, terraces, snow line, ice caps, vegetation bands; `docs/formats/surface_params.md`). Oceans are a separate, see-through surface at sea level over the real seabed, with the coastline found per pixel from the ground's height (SE's `step(height, seaLevel)`), not along mesh triangles, which shows through shallow water at its true depth; wind waves break up the sun glint (SpaceEngine's water roughness and specular defaults), and you can dive under the surface down to the seabed, with underwater fog, the sky seen through Snell's window and the depth on the HUD; ships float. Catalog planets listed without an atmosphere get the one SpaceEngine generates (gases, pressure, temperature, greenhouse, clouds and oceans from its climate model, `docs/formats/climate.md`), with its scattering model on the sky. The colour map takes the catalog's `ModulateColor` x `ModulateBright` (Mars, Venus, Titan), and airless bodies with `Hapke` keys are shaded with SE's Hapke BRDF and opposition surge (`SpotBright`, `SpotWidth`, ...), so the full Moon is evenly bright.
- **Atmospheres.** Precomputed Bruneton scattering from the shipped `.atm` tables, covering the sky from inside and outside plus aerial perspective on terrain. As in SE, the sky dims whatever lies behind it by its transmittance, per colour channel: stars and the Milky Way redden and fade towards the horizon, and the Moon and planets show in the daytime sky. A bright sky still hides the stars, galaxies and nebulae, since the port has no eye adaptation. Light is calibrated as in SE's `main-def.cfg`: the light an atmosphere scatters (sky and haze) is the star's times the atmosphere's `Bright` (`hdrAtmoScatterBright` 1), and sunlit surfaces get five times the star's light (`hdrPlanetSurfaceBright` 5). Clouds are lit as SE lights a cloud layer (`planet.glsl` ACLOUDS): the star's light is reddened through the atmosphere and cut by the planet's shadow at the cloud's height; sky light (times `SkyLight`), `DayAmbient` (0.07 by default), the `Hapke` mix and `SubsurfScatBright`/`SubsurfScatPower` follow; haze lies over the clouds, and the night side is unlit. Each atmosphere's `Opacity` mixes its transmittance towards 1, `Hue` and `Saturation` shift its scattered light (Uranus, Neptune), and the ground's and clouds' sky light is SE's ambient term: the sky irradiance times `SkyLight` plus `DayAmbient`, at the star's own light (`hdrAtmoIllumBright` 1). Earth's clouds use their map as relief (`BumpMap`, 0.5 km). Every `Clouds` block is its own layer (Titan's three orange haze layers, Mars's water-ice, dust and high procedural ones) with its own height, map, opacity and tint; Venus's colour map is an opaque layer, turned 180 degrees (`FlipMap`) and drifting with its 264 km/h wind (`Velocity`) as time runs. Each atmosphere is as thick as its body's visual height (the catalog's `Height`, else SE's formula from pressure and scale height): Titan's haze reaches 200 km, Jupiter's 300 km.
- **Star points and bloom.** SE's star model (`star_sprite.glsl` and the exe): a star's pixel brightness follows its apparent magnitude from `StarMagnLimit` (7) and `PointOverbright`, compressed above 100 and faded out at the limit, on a Gaussian of `PointScale` scaled to the screen height; dim stars lose their colour (`PointDesaturation`), and `StarColorSaturation` sets the rest. The glare of bright stars comes from SE's bloom (`hdr_down`, `hdr_gauss_blur`, `hdr_combine`): a six-level chain of blurred averages, added at `BloomBright` 0.5 before tone mapping. The settings keys are SE's; the port's earlier `StarMagLimit`, `StarBrightness`, `StarSize` and `StarSaturation` are ignored. `StarColorShift` (Settings; + red, - blue) recolours the stars as SE does, at their temperature x 10^-shift; the Graphics window has SE's bloom switch, brightness and threshold (`EnableBloom`, `BloomBright`, `BloomThreshold`).
- **Rings and models.** Rings lit as SE lights them: the lit side by phase angle with Saturn's opposition spots, the dusty side forward-scattering (Jupiter's ring brightens backlit), both through the ring's optical depth with its self-shadow, so from the unlit side the dense B ring is dark and the thin rings glow; planet shadow; rings without a texture are generated from the catalog's procedural ring keys. Asteroid and comet `.sm` models.
- **Procedural universe.** A procedural star field streamed from a magnitude-stratified octree, with SpaceEngine-style "RS" names, in whichever galaxy the camera is in: the Milky Way's analytic model, or for any other galaxy (catalog or procedural) one stretched to its size, shaped by its Hubble type (disk and bulge, elliptical, irregular), given the spiral arms of its sprite model's emission map (what SE's interior generator samples) and, as in SE, filled like a Milky Way of its size; its stars already show from 3 radii out, and new blocks of stars (and of procedural galaxies) fade in over SE's `StarFadeDuration`. Approached stars become full systems as SE's generator makes them: about half become multiple systems (binaries, triples and quadruples with SE's companion odds, white-dwarf, neutron-star, black-hole and brown-dwarf companions, log-normal separations, planets around each star and circumbinary ones), and the planets are laid out by SE's rules (5-14 planet slots spaced geometrically out to the star's sphere of influence, SE's gas giant / ice giant / terra odds and log-normal masses, SE's mass-radius fits, dwarf planets carrying belts of 100+ asteroids, SE's moon counts); their planets and moons get their composition, atmosphere, ocean, clouds, temperature and class from SE's completion chain and their surface from SE's generator, with SE's terra and airless-world colour shaders (climate bands, snow line, ice caps, plants, layered cliffs) and SE's terra cloud generator (cyclones, vortices, Coriolis banding; one giant cyclone and a clear night side on tidally locked worlds); rings too. Like SE's asynchronous loader, a system is generated in the background and entered only if the camera is still nearest that star when it is ready, its bodies spawned a few milliseconds' worth per frame, so flying through the stars at kiloparsecs a second stays smooth. Catalog stars without catalog planets (Vega, every HIPPARCOS star) get their generated system on arrival, as in SE, unless `NoPlanets true`; multiple stars without planets stay empty for now.
- **Intergalactic vision.** Outside every galaxy (from one to two radii out) the faint, unresolved galaxies are brightened so the 3000th brightest looks like a magnitude-7 one, as SpaceEngine's `AutoGalaxyMagnitude` fills the sky with galaxies; galaxies big on screen keep their own surface brightness, and the boost never lifts a sprite past a moderate level, so nearby galaxies keep their colour.
- **Procedural deep sky.** Galaxies beyond the catalogs, decoded from the exe and verified against two galaxies named in SpaceEngine's saved places: a 5 Gpc octree of 10 magnitude-banded levels with a Voronoi cosmic web, streamed around the camera (about 380 000 far sprites at 50 Mpc) and promoted to 3D models when they grow large; star clusters (`RC`) and nebulae (`RN`) inside the Milky Way and nearby galaxies, with SE's group order, counts, parameter ranges and home-galaxy brightness cut (placement approximated). All are searchable by name (`goto RG 0-9-110030168-905`), switchable in the Settings window (`ShowProcGalaxies`, `ShowProcClusters`, `ShowProcNebulae`), documented in `docs/formats/procedural_deepsky.md`.
- **Deep sky.** Galaxy particle models generated from the shipped galaxy textures and `.pst` templates, with the model's full-resolution front texture drawn over them as SE does (M 51's arms and pink knots, the Milky Way's bar seen from outside), a ray-marched volume for elliptical galaxies (which SE draws without a model), ray-marched nebulae from SE's `models/custom` packs (the Spirograph, Manatee and Omega nebulae and every catalog nebula without a named model, `docs/formats/nebula_raymarch.md`), nebula models (`models/nebulae`: cube-map, cylindrical, planetary, SNR and spherical generators, decoded from the executable), and star clusters. Every cluster, catalog or procedural, is SE's star-like particle (its `Color`, or SE's default by type; open clusters at a quarter of the light) that fades out as the cluster grows from 1 to 100 px on screen, while its member stars appear from 8 px of radius (SE's switch) in SE's five-level octree: each level holds one of SE's magnitude bands and loads only near the camera, so fainter members fill in as you approach and a globular reaches millions of stars around you; catalog clusters without a magnitude, radius or age get SE's values (an open cluster without a magnitude is never brighter than magnitude 8.1 from the Sun: Pozzo 1 about 45 L_sun).
- **Nearby stars.** The active system is the one whose real star is nearest: components of a multiple system are measured where they orbit, not at the barycentre. Only the active system's own point is hidden in the star fields; every other star near the camera stays drawn (SE hides only the barycentre).
- **Thermal glow and giants.** Hot worlds glow by SE's thermal model (hot Jupiters and lava worlds on their night side); procedural gas and ice giants use SE's cloud-band generator with its stripes, turbulence and cyclones, coloured from SE's giant palettes.
- **Stars and black holes.** Star surfaces as SE builds them: each star's granulation, flows and sunspots come from SE's surface generator (granule size by surface gravity, so red giants show a few huge convection cells and dwarfs fine granulation; spots with umbra, penumbra and filaments in a latitude band, larger on cooler dwarfs), evaluated per pixel with SE's `GlowMapSun` and coloured as a blackbody, with SE's limb darkening as temperature (0.86x at the limb to 1.1x at the centre). Black holes with Schwarzschild lensing of the scene and a raytraced, Doppler-beamed accretion disk (`docs/blackholes.md`).
- **Lighting.** As in SE, a body is lit by up to four lights: the stars of a multiple system (each in its own colour,
  faded by its magnitude and scaled to the brightest, with its own ring and eclipse shadows on surfaces, clouds, skies
  and rings) and planet shine, the light nearby planets and moons reflect (earthshine on the Moon's night side, Jupiter's
  moons on its night side). The terminator is SE's sharp, height-aware horizon shadow, so peaks catch the light past it,
  and SE's ambient lighting (none by default, `>` / `<`) lifts dark bodies.
- **Eclipses and shadows.** Soft umbra and penumbra from up to four occluders per body, on terrain, clouds and the sky (the sky's sun disc is masked by the eclipsing body); atmospheres turn a lunar umbra red. Rings cast their shadow on the planet.
- **Coronae and auroras.** Streamered coronae around resolved stars (visible around the Moon at totality), correct from any distance including inside them; auroral ovals from the catalog `Aurora` blocks, on the night side.
- **Comets.** SE's particle tails: blue gas pushed straight away from the star and whiter dust curving behind along the orbit, shorter far from the star, with SE's defaults for comets the catalog leaves without tail values; every single star of a procedural system gets SE's 100-199 comets.
- **Spacecraft and flight mode.** The 29 shipped designs (merged module meshes, DXT/PNG textures, emissive maps, frame cut-outs, engine plumes) fly with Newtonian physics in f64: engine accelerations from the design, gravity of the whole system, a warp drive whose speed is limited by altitude, terrain contact, chase and orbit cameras, a flight HUD (see Flight mode above). SE's aerodynamic models (none, shuttle, arrow; its coefficient table read from the exe) act in atmospheres, with Mach and dynamic pressure on the HUD; attitude holds (prograde to normal, horizon), hold altitude, sync velocity and an autopilot that rotates to, flies to or warps to a target in SE's stages; engine, thruster, hover and warp sounds play by thrust.
- **Scripts.** SpaceEngine `.se` scripts (the main-menu scene and the tutorials) run through `--script` or the Scripts window (`docs/formats/se_script.md`).
- **Reference tools.** The wiki (object descriptions from the locale database), the star browser and the system chart. The star browser's Search tab (main toolbar "Search", or Shift+F3) is SE's search: it scans up to 10,000 systems within a radius (catalog and procedural, generating each system as a visit would), and its Filter settings pick planets or moons by class, temperature, water, atmosphere, life (organic or exotic, unicellular or multicellular, marine or terrestrial), radius and catalog or procedural systems; the last column counts the matches, lists them on hover and selects them in turn on click. With "Star" as the object it lists the nearest 10,000 stars of a type (black hole, neutron star, white dwarf, brown dwarf, giant, or a spectral class) without generating planets. The procedural star field holds neutron stars and black holes as SE's does (4% and 2% of its white-dwarf slots).
- **Constellations and grids.** SE's constellation lines between the real stars (so their shapes change when you travel), IAU boundaries and names (full or abbreviated), and the equatorial, ecliptic and galactic grids with SE's labels plus a planet's geographic grid; all off by default, with SE's keys and fades.
- **Selection pointer.** SE's: four cyan ticks turning around the selection, outside its disc, and a hollow triangle at the screen's edge pointing to a selection off screen (Settings > "Selection pointer").
- **Procedural galaxies' brightness.** SE has no galaxies at the octree's level 0 (M -32 to -28); the port dropped them, and draws the others (whose 50 kpc radius cap packs their light into too small a disc) at the surface brightness of SE's radius law, as SE does.
- **Star points style.** SE's `StarPointMethod` (Settings > "Star points style"): sprites, sprites with motion blur (SE's default) or with motion and rotation blur. The blur stretches each star and point-like galaxy along its path on screen over the last 1/60 s of camera motion (and turning), spreading its light along the trail; `MotionBlurLength` scales it.
- **Redshift.** Distant galaxies are tinted as by cosmological redshift: a 5500 K spectrum shifted by z = d / (c/H0), at constant brightness (about 10% redder at 1 Gpc, orange by 3 Gpc).
- **Lens flares.** SpaceEngine's flare, ghost and pseudo-flare presets from `Flares.pak` on the suns of the current system, faded by the sun's magnitude and hidden by bodies in front (`docs/formats/lens_flares.md`).
- **Graphics presets.** Potato to Ultra, read from the install's `config/preset-*.cfg`: MSAA, FXAA, anisotropic filtering, landscape LOD, terrain tiles per frame and volumetric quality, in the Graphics window (Settings > Graphics) with the lens flare options.
- **Exposure.** Faint background light (galaxies, nebulae, stars) recedes while a sunlit body or a star fills the view, as the eye adapts.
- **Interface and audio.** Navigation and interface as listed under Controls, SE's fonts (Noto Sans with its Medium bold, CJK and DejaVu fallbacks), object types named as SE names them ("Yellow dwarf (G2V)", "Planet (Temperate marine terra with life)"), the startup splash, the solar system browser, interface click sounds, and the soundtrack with SE's cross-fades. The window opens maximized and remembers its size, fullscreen mode and VSync. Settings persist in `%APPDATA%/SpaceEngine-rs/settings.cfg`, and places you save from the Places window go to `places.cfg` next to it, in SpaceEngine's own format; scripted runs (`--run`, `--script`, `--screenshot`) leave both untouched.

### Not yet implemented

- **Gameplay.** Everything beyond flying one ship: docking, fuel and heat, aircraft control surfaces, damage, hull specular/normal/heat maps.

See `docs/` for the reverse-engineering notes: formats, frames, procedural rules,
atmosphere and galaxy specifications.
