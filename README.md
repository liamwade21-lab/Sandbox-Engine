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
- A SpaceEngine 0.991 installation from Steam (found through Steam on any drive or library).

### Playing the download

The download (`Sandbox-Engine-VERSION-windows.zip`) is a `Sandbox Engine` folder with one program,
`spaceengine-rs.exe`. Everything it needs except SpaceEngine is built into it: on the first start it unpacks
its shaders, textures and models (about 115 MB) to `%LOCALAPPDATA%\SpaceEngine-rs-v2\assets`, then finds
SpaceEngine through Steam (Steam's folder from the registry, then every Steam library in
`steamapps\libraryfolders.vdf`, looking for SpaceEngine's app id 314650). If Steam doesn't list it, a window asks
for SpaceEngine.exe once. If something can't be found, a message says what, instead of a black screen.
A release zip is made with `bash .audit/tools/package_release.sh VERSION OUT_FOLDER`.

### The ownership check

The port reads SpaceEngine's own files, so it is only for people who own SpaceEngine on Steam. The first
time it starts, it looks for the copy Steam lists and uses it if it passes; only if there is none, a window asks
for SpaceEngine.exe (the file dialog opens where Steam says it is). It is accepted only when all of this holds:

- the file is SpaceEngine's program (its details say SpaceEngine, by Cosmographic Software);
- it sits in a Steam library, with SpaceEngine's `data` folder beside it;
- Steam's own record of installing SpaceEngine (app 314650) is in that library and says it is fully installed;
- this computer's Steam lists that library with SpaceEngine in it;
- the Steam account that owns the install has signed in to Steam on this computer.

The choice is kept in `%APPDATA%\SpaceEngine-rs-v2\ownership.cfg` and checked again at every start (it
takes milliseconds), so the window comes back only if SpaceEngine moves or is uninstalled. `--se-path` and
`SE_PATH` must name an install that passes the same check. Test runs (`--run`, `--screenshot`) open no window:
they check the copy Steam lists. Nothing is sent anywhere. It stops casual sharing, not a determined cheat:
anyone who edits files can get past it. A copy lent through Steam Family Sharing fails the last check.

## Build and run

```bash
cargo run --release -p se_engine
```

```bash
cargo run --release -p se_engine -- --se-path "D:/SteamLibrary/steamapps/common/SpaceEngine" --start Mars
```

| Option | Meaning |
|---|---|
| `--se-path DIR` (or env `SE_PATH`) | SpaceEngine install directory (it must pass the ownership check; without it, the one checked on the first run) |
| `--mr-path DIR` (or env `MR_PATH`) | Megaton Rainfall install directory, read only when the port's own copy of the files it uses (`crates/se_engine/assets/mr`, also built into the exe) is missing (optional) |
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
| env `SE_TONEMAP=NAME`, `SE_PHOTOMODE=N` | testing: start with another of SE's tone shaders (`ACES_Fitted`, `Reinhard`, ...; default AgX) / photo mode (0 HDR, 1 Manual, 2 Auto) |
| env `SE_FRAMETIME_LOG=FILE` | write one line per frame (`frame ms nearest-star`) to FILE, to find hitches |

Script commands: `goto NAME [km]`, `select NAME`, `look NAME`, `surface LAT LON ALT_KM [PITCH] [HEADING]`,
`child N [km]` (Nth planet of the current system), `nearest`, `place TEXT` (a saved SpaceEngine place),
`at X Y Z` (parsecs from the Sun), `gal L B DIST_PC`, `galactic DIST_PC`, `date YYYY-MM-DD [HH:MM:SS]`,
`rate X`, `pause`, `resume`, `timing frame|real` (dev: a fixed 1/60 s of time a frame, or real time), `switch KEY on|off` (dev: a render switch, e.g. `ShowRain`), `rs RS L-X-Y-Z-I` (jump next to a procedural star by its name), `search LY [life|bh]` (the Star browser's search, for life or black holes; `SE_BROWSER_OPEN=search` shows it), `shot FILE` (save a screenshot and carry on), `speed KMS` (assigned velocity, km/s), `fly KMS` (dev: keep flying forward at KMS km/s, fixed step per frame), `click X Y` (dev: a left click at window pixel X, Y), `go [KM]` (Go to the selection at once, setting the velocity like G; script `goto` keeps it), `fov DEG`, `move X Y Z`, `turn YAW PITCH`, `shore [M]` (dev: move to the nearest coast, M metres inland, facing the sea), `agl M` (dev: hold the camera M metres above the ground under it, `agl off` ends it), `preset NAME` (pick a graphics preset, Low to Ultra; scripted runs otherwise keep High), `perf NAME` / `perf end` (dev: a frame-time segment for `SE_PERF=FILE`), `wait FRAMES`.
Spacecraft commands: `ship NAME` (spawn a design by id or name, e.g. `ship DragonFly`, and enter flight mode),
`throttle X` (-1 to 1: main, or retro below zero), `throttle main|retro|hover X`, `warp LOG10` (engage the warp drive at that boost, `warp off`), `flight on|off`, `unship`,
`shipcam N` (chase preset N, past the last one is the orbit view), `shiporbit YAW PITCH [DIST_M]`,
`shiprcs X Y Z` and `shipturn PITCH YAW ROLL` (held commands, -1 to 1). Example:
`--run "goto Earth 8000; ship DragonFly; throttle 0.5; wait 200"`. Env `SE_SHIPS_WINDOW=1` opens the Ships window at start.
Fleet commands: `ship build|fleet|other NAME`, `ship take|remote|teleport|destroy N`, `ship destroyall`, `ship nick N TEXT`
(N a number or nickname); `ship NAME` keeps the ships already built.

SpaceEngine scripts (`.se`, see `docs/formats/se_script.md`) run with `--script`, or from the **Scripts**
window, which lists the ones shipped in `data/scripts` (menu scene, tutorials). Supported: `GotoLocation`,
`Goto { Height/DistKm/DistRad Lat Lon Yaw Pitch Roll Time Center }`, `Select`/`Unselect`, `Center`, `Follow`,
`SyncRot`, `Free`, `Orbit`/`StopOrbit`, `FOV`, `SpeedKm`, `TimeScale`, `Date`, `StopTime`/`StartTime`, `Wait`,
`WaitTrigger` (+ `BeginMultiTrigger`/`EndMultiTrigger`) for the camera, selection, time and distance events,
`ShowMessage`/`WaitMessage`/`Print` (shown as an on-screen caption), `Set`/`SetForce`/`WaitVar`/`SaveVars`/`RestoreVars`/
`Interpolate`, `If`/`Else`/`Loop`, `Run`, and SE's `Get`/`Reset` on about 60 settings (point sources, overlays and filters, grids,
bloom and flares, the feature switches, magnitude limits, colour correction, exposure). Also `Waypoint`/`DeleteWaypoint` (drawn at
their size, selectable, with approach triggers), the tutorial GUI hooks (`HighlightWidget` blinking, message-box position,
`HideAllToolbars`, screen fades), input locks (`UserMoveControl`/`UserRotationControl`/`UserTimeControl` Disabled or Limited,
released when the script ends), music commands, the camera commands `Fly`, `Turn`/`StopTurn`, `StopFly`, `StopRotation`, `Speed`,
`Horizon`, `CheckVersion`, and `Goto`'s flight profile (`AccelTime`/`DecelTime`/`AccelPower`/`DecelPower`, `GotoSpeed`). Triggers
on GUI the port lacks (universe map, system chart, dialog clicks) time out after 4 s; all eight tutorials run to the end.
Scripted runs (`--script`) do not read or write the saved settings. Env `SE_SCRIPTS_WINDOW=1` opens the Scripts window at start.

User files: settings (`settings.cfg`), the log (`se.log`), the saved session (`save.cfg`), saved places, ships and
tutorial progress live in `%APPDATA%\SpaceEngine-rs-v2`. On its first run version 2 fills that folder with a copy of
version 1's `%APPDATA%\SpaceEngine-rs`, which it never writes to; after that the two versions keep separate settings.
Screenshots go to `Pictures\SpaceEngine-rs` for both.

Tests (engine-independent core, about 190 tests):

```bash
cargo test -p se_core
```

## The sandbox (first person)

The game starts on **Earth's ground in first person**, in whichever of eight open places has the Sun highest at that
moment (Kansas, the Pampas, Salisbury Plain, the Serengeti, the Kazakh and Mongolian steppes, New South Wales, the
Canterbury Plains). Movement, weapons, sounds and the spawn menu are Garry's Mod's and Half-Life 2's: the movement is
Source's own rules (`sandbox/movement.rs`, from the Source SDK's `gamemovement.cpp` with Garry's Mod's numbers) at 66
ticks a second, and the props, textures and sounds are read from your **Garry's Mod install** while the game runs
(found through Steam, or `GMOD_PATH`; nothing of it is copied or changed). Without Garry's Mod it still works, with
plain props and plain sounds.

| Input | Action |
|---|---|
| W/A/S/D, mouse | walk (200 units/s), look |
| Shift / Alt / Ctrl / Space | run (400) / walk slowly (100) / duck (60, and in the air a crouch jump) / jump |
| Hold **Q** | the spawn menu (let go to close it). **Spawnlists**: Garry's Mod's own lists; click a prop to spawn it where you look. **Weapons**: click one to take it. **Mods**: click who to play as (the Viltrumite hero, or yourself again) |
| 1-4, mouse wheel, then left button | pick a weapon (Half-Life 2's selection: the number steps through its slot, the left button takes it) |
| Left / right button, R | fire / second fire, reload |
| E | pick a light prop up in your hands (35 kg or less, within arm's reach) and put it down again; the left button throws it |
| Z | undo the last prop |
| **V** | free flight on and off: the port's own SpaceEngine flight with all its keys and panels, exactly as it was. V again puts you back on foot where the camera is (you fall from there). V does nothing else |

Weapons: the crowbar; the gravity gun (right button picks a prop up or drops it, left launches it or punts what is
in front); the physics gun (hold left to carry a prop at any distance, the wheel moves it nearer or further, E and the
mouse turn it, the right button freezes it in the air, R lets frozen props go); the pistol, the SMG and the shotgun
(they push props about). What you hold is Half-Life 2's own first-person model with its hands, playing its own
animations (drawing, firing, reloading, the shotgun's pump, the crowbar's swings, the gravity gun's prongs opening on
what it can lift), with Half-Life 2's view bob, sway and recoil, and its rules: how fast each fires, how the pistol's
aim spreads when you fire fast, how the SMG climbs.

Sounds are played as Half-Life 2's own sound scripts say (`sandbox/soundscript.rs`): which recordings a named sound
has, how loud, at what pitch, how far it carries. Each recording of a set plays before any plays again, a new shot
cuts the last one off (the SMG's rattle), and what you walk on, shoot or drop a prop on sounds as that surface does
(grass, dirt, concrete in a town, a wooden trunk, a metal barrel). Sounds that happen in the world come from where
they are, to the left or right.

**Buildings and trees are solid** (`sandbox/solids.rs`): you can't walk through a wall or a trunk, bullets stop at
them, props rest against them, and you can stand on a roof (V, fly over it, V again). It is the Viltrumite mod's list
of the buildings round the hero, asked for round you (within 45 m; trees within 35 m).

Gravity is the planet's own, for you and for the props: on the Moon (`V`, fly there, `V` again) a jump goes 4 m up
and props fall slowly. Props stand on the terrain as it is drawn; the props' physics is the `rapier3d` library.

The port's windows (F4 settings, F3 find, ...) want the mouse: press V for free flight first, where everything is as it
was. Garry's Mod's flashlight (F), chat, voice and context menu (C) have no counterpart here.

Not there yet: props have no shadows, you walk on the sea, props more than 60 m from you wait frozen until you come
back, a tree is solid as its trunk only (you walk under its crown), and a building you are set down inside lets you
walk out before it turns solid. Without Garry's Mod the weapons are plain shapes.

`SE_SANDBOX=0` starts the port as it was before (free flight at Earth, the saved session restored);
`SE_SANDBOX_SPOT=N` picks the starting place (0-7). Test runs (`--run`, `--screenshot`) keep the old start unless
`SE_SANDBOX=1`.

## Controls

These are the free flight's (V); on foot the keys above apply.

| Input | Action |
|---|---|
| Left drag | look around |
| Right drag | orbit the followed object (after Go to), or the selection |
| Left + right drag | zoom: move towards / away from that object |
| Shift + left drag, Page Up / Page Down | change the field of view (1 arcsecond to 120 degrees, zooms animate over 0.25 s); middle click zooms to 20 degrees and back, Home resets it. Zooming works like a telescope: stars, planets and clusters drawn as points get `(FOV / 45°)^-1.5` brighter, so fainter ones show (loaded inside the view down to magnitude 20) |
| `]` / `[`, Ctrl+`]` / `[`, P | raise / lower the magnitude limit by 0.1 (all, or stars only), reset it to the startup value |
| Shift+`]` / Shift+`[` | galaxies' and nebulae' magnitude limit (`]` / `[` change it with the stars', P resets) |
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
| Space | pause time; K/L (and the navigation toolbar's << / >>) step the rate through SE's 1, 3, 10, 30, 100, ... (0.3, 0.1, ... below 1) and resume a paused clock; J reverses; `\` sets real time; Ctrl+`\` the current date; click the date to type one; Shift+`\` switches between real time and SE's frame time mode (1 / `RealTimeFPS` per frame, for recording) |
| M / N | toggle music / next track (tracks cross-fade, the title shows on screen) |
| F2 / Ctrl+F2 / Alt+F2 | solar system browser at the selection's level / the camera's system / the system's root |
| Alt+Enter / Shift+Enter / Ctrl+Enter | fullscreen / next display / minimize |
| F1 / Ctrl+` | the list of controls (F1 is SE's universe map, not ported) / hide the interface |
| F3 / I | SE's Find object dialog (Enter selects) / the wiki: the selected object's description from SpaceEngine's locale database |
| Shift+F3 / F8 or 0 | star browser (nearest stars, and the search for planets, moons and stars by parameters; also the main toolbar's Search button) / system chart of the current system |
| F4 / Ctrl+F4 / F7 | settings (SE's tabs: Graphics, Camera, Display, Interface, Sound, and Reset settings) / the same on Graphics / locations |
| F11 | screenshot: `scrNNNNN.jpg` in Pictures/SpaceEngine-rs (jpg, png, dds, tga or tif, JPEG quality and alpha in Settings > Display) |
| B (outside flight) | photo mode: HDR, Manual, Auto (SE's histogram auto exposure). SE's key for it is V, which here is the sandbox's free-flight switch |
| O / ' / Ctrl+' / ; | orbits / labels (brief, detail, off) / markers / selection pointer; each kind of object can be switched in Settings > Filter objects |
| X / Shift+X / Ctrl+X / Alt+X | constellations / their lines / boundaries / names (right click the Constellations button for the switches) |
| Ctrl+; / Alt+; | celestial grids (equatorial by default; ecliptic and galactic on the Celestial grids button's right click) / a planet's geographic grid |
| Shift+. / Shift+, (`>` / `<`) | SE's ambient lighting up / down in steps of 0.05 (none by default), so dark bodies can be seen |
| Shift+A / Shift+C / Shift+Ctrl+A / Shift+Ctrl+C / Ctrl+L / Shift+L | show or hide atmospheres / clouds / auroras / comet tails / eclipse shadows / planet shine (SE's keys, saved with the settings) |
| `.` / `,` / `/` | exposure compensation up / down by a quarter stop (repeats while held; -4 to +42) / reset |

### Flight mode (spacecraft)

Open the **Ships** window (button at the top right, below the bar), pick a design and press **Fly**: the ship appears
where the camera is (its chase camera at the camera's position), at rest relative to the nearest body, and flight mode
starts. Ship data comes from SpaceEngine's `ShipsModels.pak`; see `docs/formats/spacecraft.md`.

| Input | Action |
|---|---|
| Enter (or F outside flight) | toggle flight mode (needs a ship) |
| Esc | leave flight mode; the free camera stays where the chase camera was and the ship keeps drifting |
| `=` / `-` (keypad `+` / `-`) | main engines up / down; with Shift the retro engines, with Ctrl the hover engines, with Alt the warp drive (SE's keys) |
| Z / Alt+Z / Shift+Z | stop the main, retro and hover engines (and any autopilot program) / stop the warp drive (SE's "Stop hyperdrive") / sync velocity with the reference body |
| W / S / A / D (or arrows), R / F (or keypad 1 / 0) | thrusters forward / back / left / right, up / down |
| Keypad 8 / 2 / 4 / 6, Q / E (or keypad 7 / 9) | pitch and yaw, roll (`TurnEngines`); Shift is ×10 |
| Mouse drag | turn the ship (chase views) or orbit it (orbit view); wheel zooms the camera |
| U / Numpad 5 | flight assist (rotation damping, on by default) / kill rotation |
| 5 / 6 / 7 / 8 / 9 / 0, End | hold prograde / retrograde / radial / antiradial / normal up / normal down, horizon (relative to the reference body) |
| H | hold altitude |
| T / P | select the camera's selection as the target / cycle the reference body (nearest, strongest gravity, the target) |
| C / Shift+G / Alt+G / G | autopilot: rotate to / fly to / warp to / go to (fly or warp) the target; Z stops it |
| PageUp / PageDown, Alt+`=` / Alt+`-` | warp drive (ships with a `Warpdrive` or `Hyperdrive`): engage and raise the target boost / lower it, stopping at 0 |
| V | cycle the design's `Behind` chase cameras and the free orbit camera |
| Ctrl + keypad 5 / 4 / 6 / 8 / 2 (held), Alt+O, Alt+C | look back / left / right / up / down from the chase or cockpit view; the orbit view; centre the orbit view behind the ship (SE's keys) |
| Ctrl+1 .. Ctrl+5 | flight HUD: Off / Horizontal / Orbital / Warp / Docking (SE's pointers in the skin's colours; script `flight hud MODE`) |

Engines are accelerations in m/s² (not thrust), warp multiplies velocity by 10^boost with the boost capped as the
ship nears a body, so warp speed grows with altitude. Gravity comes from every body of the system, and the ground is the
terrain (touchdown pins the ship to the rotating planet; 25 m/s or more is a crash notice). The HUD shows ship, speed
relative to the nearest body, throttle, warp state and altitude. The Ships window also toggles gravity and collisions.

The interface follows SE's layout: the selection's info table as text in the upper-left corner
(diameter, magnitudes, luminosity, gravity, density, albedo, temperature, pressure, orbit, rotation,
apparent size, the ground under the camera; brief or full, saved as `TextMode`), and four toolbars:
tools (Find object, **Places** with SpaceEngine's own saved places, **Scripts** for the shipped `.se`
scripts, Search, Wiki, **Settings**, **Graphics** for quality presets, bloom and lens flares, Help, **Mods**)
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

## Mods

The **Mods** button (tools toolbar, bottom left) lists the mods built into the game: one folder each under
`crates/se_engine/assets/mods`, each with a `mod.cfg` (name, description, the files it needs). Click a row to turn
that mod on or off. A row is grey, with the reason, when one of its files is missing or bad, when it needs a newer
build, while you pilot a ship or while a script runs; starting to pilot or a script turns the mod off first. The
first mod is **Viltrumite powers** (plan: `docs/spec-mods-viltrumite-powers.md`): you play Omni-Man from behind, flying. He appears where the camera looks; turn the mod off and the camera stays where it is. So far he flies, falls, lands, walks, runs and jumps, with a shadow, a cape that moves and his sounds (the air going past by real air pressure and speed, per planet; silent where there's no air, in space and on airless ground; at the Sound effects setting), booms and heats in the air, punches, punches the ground and claps, and hard landings leave craters; buildings he hits hard sink into the ground and leave rubble, back once he and the camera are 20 km away. The row reads Loading for the moment before he appears, and a damaged file greys it. 

While he is on the panels are hidden and a small readout (speed, Mach in air, height, gravity, heat) stands in the corner:

Since 2026-11-07 the controls are the ones of your own Viltrumite mod (`C:\Users\liamw\Desktop\Viltrumite`); a line at the bottom of
the screen shows the keys for what he's doing, a cross marks the middle and a bar under it shows a charge.

| Input | What he does |
|---|---|
| Mouse | Looks; in fast flight it steers. Below the atmosphere's height the view keeps the horizon level, above it it's free |
| Space on the ground | Tap: take off into a hover. Hold: charge (a crouch, a rumble, the charge sound, the bar), let go: launch into fast flight where you look, at least 15 degrees up, faster the longer it charged. A full charge (1.4 s) breaks the sound barrier: a boom |
| Ctrl+Space on the ground | Super jump, 28 m/s up: 40 m on Earth, 242 m on the Moon |
| W, A, S, D hovering | Move at 20 m/s where you look (look up and W to rise); Ctrl goes straight down and sets him down on the ground |
| W twice | From a hover into fast flight |
| Space hovering | Tap: drop (real gravity and drag; an orbit if he's fast enough). Hold, let go: a launch into fast flight where you look |
| W, S in fast flight | Faster (300 m/s after 1 s, 3 km/s after 3 s, 30 km/s after 6 s; in air that's the limit; in space it doubles every second: light speed about 13 s later, 1000 c about 23 s later, easing near anything), slower. No keys: he keeps his speed. Over the ground he skims along it, never down into it |
| S twice | Out of fast flight into a hover |
| Ctrl in fast flight | Dive straight down into a cratering landing; let go before the ground to pull up into a hover |
| Space held past the barrier | When the bar is full (0.7 s) let go: a boost stack, 1.5 times the speed and a boom each, up to 5. S, a dive, a hover or dropping under the barrier ends them |
| Shift + W on the ground | Run, 5 m/s; held 1.5 s, sprint, 11 m/s. Shift held and W twice: a super run at 80 m/s with a boom at its start; tap Space while running and he takes off into fast flight at that speed |
| X | Drop, as a tap of Space hovering |
| Space while falling | Stops the fall at once: he hovers where he is |
| Z | Stop dead, in the air; on the ground he just stops walking |
| Wheel | Camera nearer or further (2.5 to 200 m) |
| Tab | The menus: the panels come back and the mouse goes to them, the keys stay his. Tab again to go back. The same happens by itself when the window loses the focus |
| G | Lock on to the selection (G again lets go); then hold W and he flies there himself, faster the further it is, and eases in to stop in a hover near it. Anything more than 5 minutes away can't be locked on |
| Shift+G, Ctrl+G | Land on the selection, go to its centre, as usual: it carries him; WASD or Z stop it |
| Left button in supersonic fast flight | Select what's under the cross (a planet, a moon, a star, a galaxy) instead of punching; too far (over 5 minutes away) and it says so and keeps the old selection |
| Left button | Click: a punch (a 5 m reach); quick clicks chain a combo, the fourth the superman punch. Hold, then let go: a charged punch (from 0.55 s, full at 2.2 s, the bar shows it) that blows up where it lands, a crater where that's near the ground. Aimed 45 degrees or more down within 15 m of the ground, a click is a ground punch: down to the ground and a 6 m crater |
| Right button | Hold to charge the thunderclap (0.4 to 2.5 s), let go to clap: a ring of air 30 to 150 m in radius by the charge, and dust kicked up when he's over the ground |

Breaking the sound barrier (by real physics, or a boost stack, or a super run's start) shakes the camera and widens the view for a
moment; fast flight widens it a little with speed and each stack. Past the barrier in air the view buffets steadily (more with Mach
and each stack) and faint speed lines run out from the middle of the screen. The air rushes past by his speed in any flight, and past
the barrier (or in a dive) the roar plays too. All his sounds are twice as loud as before (your note: too quiet). Each boost stack
also lifts the speed limit near planets (30 km/s in the air, 1.5 times a stack).

The lock-on: past the sound barrier in fast flight, put the cross on a planet, moon or star and click to select it, press G to lock
on, and hold W. The view swings round to it and he speeds up by how far it is (doubling every quarter second, up to ten million times
light's speed), then slows smoothly to stop in a hover 1.5 radii from a planet's or moon's middle (10 radii from a star's). A body in
the way, its air with it, he climbs over at 55 degrees first. A gold mark shows the target, the distance left and the time to get
there (an arrow when it's off the screen). Let go of W and he coasts, never faster than the lock allows that close; S brakes, Z stops
and lets go of it. Earth to the Moon takes about 10 s, Alpha Centauri about 35; anything over 5 minutes (about 80 light years) can't
be selected this way or locked on. The numbers are `LOCK_*` at the end of `tune.rs`.

The air does what its physics says, on every world by its own air. Past the speed of sound (Earth about 335 m/s at sea level, Mars 233, Venus 449) a sonic boom goes off with a ring of cloud, and a cone of vapour hangs round him up to Mach 1.6; slowing back down it ends with a softer sound and a puff of the cone, no boom. Above the air, 55 km on Earth, 16 on Mars, 188 on Venus, there is no boom and no cone, so coming in from orbit doesn't boom, and a world with under 150 Pa of air never does. Faster still his skin heats: dull orange at 800 K (about 1 km/s at sea level on Earth, 7 times less speed on Venus), yellow at 2,000, white with a shell of plasma round him at 3,500, a blue-white trail from 6,000, and at 10,000 K (9.7 km/s at sea level) the look goes no further; only the shell and the trail get bigger. The heat bar in the readout shows it, in its colour. He cools over seconds when he slows. Neither hurts him or slows him. The trail lies behind the camera in a straight line: you see it when you turn, or with the view off to the side. A Go To through the air booms and heats him too, by its real speed through the air: a landing from orbit or a Go To leaving from near the ground makes him white-hot for a moment.

He plays on the real clock: the same at x1000 and when paused, and while hovering he holds his place over the ground whatever the clock does. He lands by how fast he comes down: under 12 m/s he's straight on his feet, up to 35 m/s a short stop, faster a hard landing with a crater (2.4 m in radius at 35 m/s, 12 m at 900 m/s, 60 m at most), broken ground round him and a burst of dust in the ground's own colour; on the sea a white splash instead. A crater stays until both he and the camera are 20.5 km away (it doesn't fade before), also after the mod is turned off; at most 32 at a time, the farthest giving way. Big impacts shake the view a little. On foot he walks up slopes to 50 degrees and slides down steeper ones; steeper than 70 is a wall, which he falls past (a long one ends in a crash at the bottom), and he falls off ledges. Over the sea he stops 1.5 m up and hovers (fly to land to walk on); a gas giant, a star, a black hole or a comet model only stop him: he hovers there too. His shadow falls the way the sun says, fades as he climbs (gone by 200 m), under cloud and at dusk. Starting to pilot a ship or a script turns the mod off first. Mods are always off when the game starts. `SE_MODS=0` hides the button, for comparisons.

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
- **Planet surfaces.** Textured planets use SpaceEngine's tiled cube maps: colour, 16-bit relief, oceans with depth colour and glint, night lights and a separate cloud shell (no cloud shadows on the ground, as in SE 0.991). A quadtree LOD with skirts and horizon culling drives them, and procedural detail is added below the data resolution; every level samples the same surface (SpaceEngine's generator has fixed octave counts), so coasts and lakes stay put as patches split; each patch carries a mipmapped, anisotropic normal map at four times its mesh resolution (SE: 256 texels on 33 vertices), so relief finer than the mesh shades from afar, and a freshly split patch starts on its parent's shape and morphs into its own as the camera nears it. Height tiles are read with SE's Catmull-Rom filter, and the normal map's lookups are jittered by up to half a texel as SE's are. Every rocky world is textured like SpaceEngine's: a biome preset (`textures/terrain/samples`, chosen as SpaceEngine does from its body classes, star temperature and generated `SurfStyle`, `docs/formats/biome_presets.md`) gives tints and tiled sample textures per biome (sea floor, shelf, beach, lowland, upland, rock, snow, ice, lava, vegetation), blended by height, slope and climate; close to the ground the preset's soil materials (`textures/terrain/materials`; a layer's frequency is its repeats per 128 m, which puts grass at about a metre per repeat and sand ripples under a metre apart, as SpaceEngine shows them from 1.5 and 20 m) add centimetre-to-metre detail at their full 1024 px, each point taking the soil SpaceEngine's own rule picks (its decrypted `tg_detail_maps` shader: HSV distance of the soil's colour to the colour map plus the slope gap, `docs/formats/se_shaders.md`; Earth's grassland gets grass, the Sahara rippled sand), recoloured to the ground in HSV as SE does, with their normal, roughness and occlusion maps, full within 1 km and gone by 2 km (SE's fade); the sample layers add their own relief (a normal map made from each sample's height map, and its roughness) from about a hundred metres to orbit. Each layer fades in as its repeat grows on screen, so no tiling shows. SE's tiles carry a 2-texel border shared with their neighbours, sampled as such (no cliffs along tile edges), and the LOD measures distance to the ground's real height, so patches under the camera on high ground split fully. Procedural worlds take their colours from the biomes (airless and icy ones also SE's rayed crater ejecta, ice cracks and freckles); textured ones keep their maps and gain the detail. Smooth discs shade without a grid (normal maps store each texel's tilt from the smooth sphere). City lights switch on by the light actually reaching the ground (they come on in an eclipse or ring shadow and a bright moon turns them off, as in SE), Io's `GlowMode "Permanent"` lava shines by day, and ice (`DiffMapAlpha "Ice"`) has its own glint. Relief and climate of worlds without a bump map follow the surface parameters SpaceEngine generates for each body (mountains, dunes, hills, canyons, craters, maria, terraces, snow line, ice caps, vegetation bands; `docs/formats/surface_params.md`). Oceans are a separate, see-through surface at sea level over the real seabed, with the coastline found per pixel from the ground's height (SE's `step(height, seaLevel)`), not along mesh triangles, which shows through shallow water at its true depth; wind waves break up the sun glint (SpaceEngine's water roughness and specular defaults), and you can dive under the surface down to the seabed, with underwater fog, the sky seen through Snell's window and the depth on the HUD; ships float. Catalog planets listed without an atmosphere get the one SpaceEngine generates (gases, pressure, temperature, greenhouse, clouds and oceans from its climate model, `docs/formats/climate.md`), with its scattering model on the sky. The colour map takes the catalog's `ModulateColor` x `ModulateBright` (Mars, Venus, Titan), and airless bodies with `Hapke` keys are shaded with SE's Hapke BRDF and opposition surge (`SpotBright`, `SpotWidth`, ...), so the full Moon is evenly bright. Near the camera the soils also raise and lower the ground itself (SE's "3D stones", `DisplaceDetailTextures` / `DisplaceDetailHeight`): within their 2 km each vertex moves by its soils' height maps, 0.1 m per metre of the soil's repeat (rippled sand by about half a metre).
- **Atmospheres.** Precomputed Bruneton scattering from the shipped `.atm` tables, covering the sky from inside and outside plus aerial perspective on terrain. As in SE, the sky dims whatever lies behind it by its transmittance, per colour channel: stars and the Milky Way redden and fade towards the horizon, and the Moon and planets show in the daytime sky. A bright sky still hides the stars, galaxies and nebulae, since the port has no eye adaptation. Light is calibrated as in SE's `main-def.cfg`: the light an atmosphere scatters (sky and haze) is the star's times the atmosphere's `Bright` (`hdrAtmoScatterBright` 1), and sunlit surfaces get five times the star's light (`hdrPlanetSurfaceBright` 5). Clouds are lit as SE lights a cloud layer (`planet.glsl` ACLOUDS): the star's light is reddened through the atmosphere and cut by the planet's shadow at the cloud's height; sky light (times `SkyLight`), `DayAmbient` (0.07 by default), the `Hapke` mix and `SubsurfScatBright`/`SubsurfScatPower` follow; haze lies over the clouds, and the night side is unlit. Each atmosphere's `Opacity` mixes its transmittance towards 1, `Hue` and `Saturation` shift its scattered light (Uranus, Neptune), and the ground's and clouds' sky light is SE's ambient term: the sky irradiance times `SkyLight` plus `DayAmbient`, at the star's own light (`hdrAtmoIllumBright` 1). Earth's clouds use their map as relief (`BumpMap`, 0.5 km). Every `Clouds` block is its own layer (Titan's three orange haze layers, Mars's water-ice, dust and high procedural ones); planets whose clouds SE generates get its 1 to `PlanetCloudsTerra` (3) layers, at its heights, reliefs and winds, each with its own pattern (SE moves each layer's `Randomize`) with its own height, map, opacity and tint; Venus's colour map is an opaque layer, turned 180 degrees (`FlipMap`) and drifting with its 264 km/h wind (`Velocity`) as time runs. Each atmosphere is as thick as its body's visual height (the catalog's `Height`, else SE's formula from pressure and scale height): Titan's haze reaches 200 km, Jupiter's 300 km.
- **Volumetric clouds** (version 2, `docs/spec-volumetric-clouds-weather.md`, in progress). On Ultra (half resolution) and High
  (quarter) the planet the camera is at draws its SE cloud layers as ray-marched 3D clouds: cover from SE's cloud generator or the
  catalog cloud maps, low cumulus to high veils, lit through the atmosphere with sky light and haze, casting shadows on the ground.
  Weather shapes that cover: belts that follow the seasons, storms moving east, afternoon convection (worked out from the date,
  `se_core/src/weather.rs`), and a live simulation on the GPU for the active planet (thunderstorm clusters that grow, drift with
  the winds, leave anvils and die out at night); near the camera the clouds drift with the wind. Paused, the weather stands still;
  a date jump shows the new date's weather at once. Weather effects: rain and snow (grey shafts under raining clouds, streaks or
  flakes around the camera; snow where the air is below freezing), lightning in thunderstorms (lighting the clouds and the
  ground), fog on calm humid mornings, and dust storms on dry worlds (e.g. Mars), each with its own switch.
  The 3D clouds draw the planet you are at from the ground up to space; SE's flat clouds draw planets farther than 40 to 60
  radii away, cross-fading with the 3D ones, which also fade in over a second once ready (the plan for better-looking and faster
  clouds is `docs/spec-clouds-v3.md`). The planet's cloud map is about 5 km a texel (Earth), and from a few thousand km down a stack
  of detailed maps around the camera (400 m to 1.6 km a texel, from SE's finer tiles or its cloud generator) keeps the clouds' shapes
  sharp; far-away clouds are sharpened to full resolution by the cloud map; the clouds within a few km are traced at full resolution
  on Ultra (half on High). For speed each frame traces a quarter of the cloud pixels in turn, steps quickly through clear air and
  works the far clouds' sunlight out from the cloud above them. The weather's own clouds (storms, fronts) take their shapes from a
  cloud pattern made for each planet. Test switches: `SE_CLOUD_QUARTER=0` (trace every pixel each frame), `SE_CLOUD_LOD=0` (a hard
  switch, no fades), `SE_DETAIL_COVER=0`, `SE_NEAR_PASS=0`, `SE_WEATHER_NEUTRAL=1` (no weather), `SE_WEATHER_SIM=0`,
  `SE_WEATHER_DEBUG=1` (a weather map on screen); dev command `vclouds 0|1|2` (the 3D clouds off, High, Ultra).
  Settings > Graphics > Clouds and weather sets the quality (`VolumetricClouds`) and the weather effects' switches (`ShowRain`,
  `ShowLightning`, `ShowFog`, `ShowDust`). Medium and below, other bodies, HD screenshots and the
  projection modes keep the flat cloud shells for now.
- **Vegetation and cities (being built, `docs/spec-vegetation-civilizations.md`).** The Megaton Rainfall files they use
  (trees, grass, window tiles) are copied into the project (`crates/se_engine/assets/mr`): the game need not be installed.
  The copy is for private use, not for sharing. The GPU scatter places objects on the ground and culls them on the GPU, standing
  exactly on the drawn ground at every distance; meshes up close, baked impostors to 3 km, lit and hazed as the ground.
  Planets with land plants in their catalog Life (Earth, SE's generated terra worlds with multicellular land life) get
  Megaton Rainfall's 16 trees and its grass, and trees grown by the port's generator: Earth's conifers (cold forests),
  palms (tropical coasts), acacias and shrubs (steppe), and for every generated life planet a flora of its own, shaped
  by its gravity and air. Earth's from its colour map (forests, copses in fields, lone trees on the steppe), SE's worlds
  by the ground's own biome weights. The trees take the ground's colour, the forest floor darkens
  under them and past 3 km the ground draws the treetops; trees and grass sway in the weather's wind. On by default:
  `SE_SCATTER=0` or the dev command `scatter off` turns it off, `scatter test` (or `SE_SCATTER_TEST=1`) is the even test
  cover of one tree over any land; `SE_SCATTER_DEBUG=1` logs its cells and counts.
  Civilizations: about 1 in 20 generated planets with land plants carries one (`Life { Type "Intelligent" Civilization
  1..4 }`: primitive, modern, futuristic, interstellar), Earth is level 2; the info panel names the level and the star
  browser finds them (Life type "Intelligent", a Civilization filter; dev command `search LY civ`). Their towns: villages
  of huts (level 1), modern cities with streets, lamps and traffic (level 2; Earth's from its lights map), future cities of
  megatowers, skybridges and flying traffic (level 3), and at level 4 domed colonies on the system's moons and planets,
  orbital stations and streams of ships between them (dev commands `town`, `station N KM`).
  Broadleaf trees follow the seasons (autumn colours, bare winters, spring's green) by latitude and date; trees near snow
  carry snow. Settings > Graphics > Vegetation and cities sets how far trees, grass and buildings are drawn (`Vegetation`,
  `Cities`: Off, Low, High, Ultra; dev command `levels V C`).
- **Star points and bloom.** SE's star model (`star_sprite.glsl` and the exe): a star's pixel brightness follows its apparent magnitude from `StarMagnLimit` (7) and `PointOverbright`, compressed above 100 and faded out at the limit, on a Gaussian of `PointScale` scaled to the screen height; dim stars lose their colour (`PointDesaturation`), and `StarColorSaturation` sets the rest. Bright points carry SE's flare wing (`PointFlareBright`, `PointFlareDecay`; Settings > Graphics > Point sources) around their Gaussian. The glare of bright stars comes from SE's bloom (`hdr_down`, `hdr_gauss_blur`, `hdr_combine`): a six-level chain of blurred averages, added at `BloomBright` 0.5 before tone mapping. The settings keys are SE's; the port's earlier `StarMagLimit`, `StarBrightness`, `StarSize` and `StarSaturation` are ignored. `StarColorShift` (Settings; + red, - blue) recolours the stars as SE does, at their temperature x 10^-shift; the Graphics window has SE's bloom switch, brightness and threshold (`EnableBloom`, `BloomBright`, `BloomThreshold`).
- **Rings and models.** Rings lit as SE lights them: the lit side by phase angle with Saturn's opposition spots, the dusty side forward-scattering (Jupiter's ring brightens backlit), both through the ring's optical depth with its self-shadow, so from the unlit side the dense B ring is dark and the thin rings glow; planet shadow; rings without a texture are generated from the catalog's procedural ring keys. Close up (within the ring's `VolumDrawDist`, 450 km for Saturn) SE's volumetric rings take over (`VolumetricRings`, Settings > Graphics): individual icy rocks, one per `RocksMaxSize` cell (8 m for Saturn), spread and stacked by the ring texture's density, each subring orbiting at its own Kepler speed, lit with PBR from `SnowSE001`, inside dust lit as the flat ring (SE's `rings_raymarch` shader, `docs/formats/rings.md`). Asteroid and comet `.sm` models.
- **Procedural universe.** A procedural star field streamed from a magnitude-stratified octree, with SpaceEngine-style "RS" names, in whichever galaxy the camera is in: the Milky Way's analytic model, or for any other galaxy (catalog or procedural) one stretched to its size, shaped by its Hubble type (disk and bulge, elliptical, irregular), given the spiral arms of its sprite model's emission map (what SE's interior generator samples) and, as in SE, filled like a Milky Way of its size; its stars already show from 3 radii out, and new blocks of stars (and of procedural galaxies) fade in over SE's `StarFadeDuration`. Approached stars become full systems as SE's generator makes them: about half become multiple systems (binaries, triples and quadruples with SE's companion odds, white-dwarf, neutron-star, black-hole and brown-dwarf companions, log-normal separations, planets around each star and circumbinary ones), and the planets are laid out by SE's rules (5-14 planet slots spaced geometrically out to the star's sphere of influence, SE's gas giant / ice giant / terra odds and log-normal masses, SE's mass-radius fits, dwarf planets carrying belts of 100+ asteroids, SE's moon counts and moon masses, with binary planets and massive moons around their barycentres; SE's ring roll on every cold planet, moon and asteroid; supermassive black holes of 400+ solar masses with their orbiting stars; SE's life rules by volatiles class with panspermia; stars with SE's mass, age, [Fe/H] and C/O; systems seeded from the star's position as SE seeds them); their planets and moons get their composition, atmosphere, ocean, clouds, temperature and class from SE's completion chain and their surface from SE's generator, with SE's terra and airless-world colour shaders (climate bands, snow line, ice caps, plants, layered cliffs) and SE's terra cloud generator (cyclones, vortices, Coriolis banding; one giant cyclone and a clear night side on tidally locked worlds); rings too. Like SE's asynchronous loader, a system is generated in the background and entered only if the camera is still nearest that star when it is ready, its bodies spawned a few milliseconds' worth per frame, so flying through the stars at kiloparsecs a second stays smooth. Catalog stars without catalog planets (Vega, every HIPPARCOS star) get their generated system on arrival, as in SE, unless `NoPlanets true`; multiple stars without planets stay empty for now.
- **Intergalactic vision.** SE's `AutoGalaxyMagnitude`: every drawn galaxy adds the part of the sky it covers, and the less of the sky galaxies cover, the brighter all galaxies are drawn, eased at `AutoGalaxyMagnSpeed`, up to the brightness of a magnitude-15 limit in empty space and not at all inside a galaxy; galaxies big on screen keep their own surface brightness, and the boost never lifts a sprite past a moderate level, so nearby galaxies keep their colour.
- **Procedural deep sky.** Galaxies beyond the catalogs, decoded from the exe and verified against two galaxies named in SpaceEngine's saved places: a 5 Gpc octree of 10 magnitude-banded levels with a Voronoi cosmic web, streamed around the camera (about 380 000 far sprites at 50 Mpc) and promoted to 3D models when they grow large; star clusters (`RC`) and nebulae (`RN`) inside the Milky Way and nearby galaxies, with SE's group order, counts, parameter ranges and home-galaxy brightness cut (placement approximated). All are searchable by name (`goto RG 0-9-110030168-905`), switchable in the Settings window (`ShowProcGalaxies`, `ShowProcClusters`, `ShowProcNebulae`), documented in `docs/formats/procedural_deepsky.md`.
- **Deep sky.** Galaxy particle models generated from the shipped galaxy textures and `.pst` templates (built once a galaxy is 2 px across and faded in by 5 px, as SE does, up to `CacheSizeGalModels` (64) at once), with the model's full-resolution front texture drawn over them as SE does (M 51's arms and pink knots, the Milky Way's bar seen from outside), elliptical galaxies (which SE draws without a model) as SE's `galaxy_bulge` shader draws them from its pre-integrated density table, ray-marched nebulae from SE's `models/custom` packs with SE's own `nebula_raymarch` shader and the catalog's `Randomize` (the Spirograph, Manatee and Omega nebulae and every catalog nebula without a named model, `docs/formats/nebula_raymarch.md`), nebula models (`models/nebulae`: cube-map, cylindrical, planetary, SNR and spherical generators, decoded from the executable), and star clusters. Galaxy and nebula sprite models use SE's brightness chain (`GlowBrightMin`, `GlowMagnLimit`, `hdrGalaxySpriteBright`, `hdrNebulaSpriteBright`), its nebula sprite atlas and its single far-to-near sprite pass, so dust darkens only the glow behind it; distant galaxies redden with SE's redshift ramp, and `NoModel` nebulae (M 43, the Running Man) draw nothing of their own. Every cluster, catalog or procedural, is SE's star-like particle (its `Color`, or SE's default by type; open clusters at a quarter of the light) that fades out as the cluster grows from 1 to 100 px on screen, while its member stars appear from 8 px of radius (SE's switch) in SE's five-level octree: each level holds one of SE's magnitude bands and loads only near the camera, so fainter members fill in as you approach and a globular reaches millions of stars around you; catalog clusters without a magnitude, radius or age get SE's values (an open cluster without a magnitude gets SE's built-in cap of magnitude 7.1 from the Sun: Pozzo 1 about 114 L_sun). Members come from SE's own generator (its class tables, masses, ages and remnants, open clusters filled by their light budget, dense ones by SE's tries per octree node with the `CenPow` profile), globulars and galactic nuclei have SE's central black hole, member stars carry SE's names (`RSC 8514-29-...`) and can be found by name, and the info panel counts the loaded members.
- **Nearby stars.** The active system is the one whose real star is nearest: components of a multiple system are measured where they orbit, not at the barycentre. Only the active system's own point is hidden in the star fields; every other star near the camera stays drawn (SE hides only the barycentre).
- **Thermal glow and giants.** Hot worlds glow by SE's thermal model (hot Jupiters and lava worlds on their night side), and their atmospheres glow too, scattered as if lit from the planet's centre (SE's GlowColorAtmo); procedural gas and ice giants use SE's cloud-band generator with its stripes, turbulence and cyclones, coloured from SE's giant palettes.
- **Stars and black holes.** Star surfaces as SE builds them: each star's granulation, flows and sunspots come from SE's surface generator (granule size by surface gravity, so red giants show a few huge convection cells and dwarfs fine granulation; spots with umbra, penumbra and filaments in a latitude band, larger on cooler dwarfs), evaluated per pixel with SE's `GlowMapSun` and coloured as a blackbody, with SE's limb darkening as temperature (0.86x at the limb to 1.1x at the centre), and as bright as SE draws them (`hdrStarSurfaceBright` through SE's temperature curve and Planck table: the Sun about 7 at the centre). Oblate stars (`Oblateness`, `Dimensions`, `DimensionsSol`: Altair, Vega, Achernar) are ellipsoids. Coronae are SE's three planes of animated fbm rays per star, sized and tuned by the catalog's `Corona` block or SE's defaults (none with `NoCorona`); brown dwarfs show SE's glowing cloud bands over the photosphere. Black holes as SE's GR shader draws them: Kerr-Newman lensing (`KerrSpin`, `KerrCharge`: the flattened, shifted shadow of a spinning hole) with SE's volumetric accretion disks and jets (its DiskProperties and JetProperties, Planck emission, Doppler beaming, the catalog's keys with SE's defaults); rays that leave the screen take the real sky from an environment cube map, and holes with a catalogued disk `Luminosity` shine as stars from afar; neutron stars and white dwarfs with an `AccretionDisk` are drawn by the same shader, with their light-bent surface (`docs/blackholes.md`).
- **Lighting.** As in SE, a body is lit by up to four lights: the stars of a multiple system (each in its own colour,
  faded by its magnitude and scaled to the brightest, with its own ring and eclipse shadows on surfaces, clouds, skies
  and rings) and planet shine, the light nearby planets and moons reflect (earthshine on the Moon's night side, Jupiter's
  moons on its night side). The terminator is SE's sharp, height-aware horizon shadow, so peaks catch the light past it,
  and SE's ambient lighting (none by default, `>` / `<`) lifts dark bodies.
- **Eclipses and shadows.** SE's eclipse model (eclipse_common.glh): soft umbra and penumbra on terrain, clouds and the sky, and up to 8 casters per light (stars included) on the sky and rings; the sky is shadowed per light and per pixel, so in totality the zenith darkens while the horizon stays lit. Rings cast SE's `RingsShadow` on the planet and its sky (density from the catalog's `PlanetShadow`, anti-aliased, fading at equinox); the planet, flattened as it is, and its moons shadow the rings, and the rings are lit by their planet's own light.
- **Coronae and auroras.** Streamered coronae around resolved stars (visible around the Moon at totality), correct from any distance including inside them; auroral ovals as SE draws them (noise-displaced sprite particles on rings, SE's defaults by body class, no day/night mask) from the catalog `Aurora` blocks.
- **Comets.** SE's particle tails: blue gas pushed straight away from the star and whiter dust curving behind along the orbit, shorter far from the star, with SE's defaults for comets the catalog leaves without tail values; every single star of a procedural system gets SE's 100-199 comets.
- **Spacecraft and flight mode.** The 29 shipped designs (merged module meshes, DXT/PNG textures, frame cut-outs) are drawn with SE's hull material (normal, bump and two-scale detail maps, specular, emissive, the thermal glow of `RadTemp` designs, lit by up to four stars with eclipse shadows) and SE's ray-marched exhausts (its eight engine shapes); main, retro and hover engines have their own levels on SE's keys. They fly with Newtonian physics in f64: engine accelerations from the design, gravity of the whole system, a warp drive whose speed is limited by altitude, terrain contact, chase and orbit cameras, SE's flight HUD modes with its panel rows (see Flight mode above), the Ships window's My ships / Other ships tabs and Build menu (build a ship or a fleet, take control, control remotely, teleport, destroy, nickname; idle ships hold station by their body), and SE's Alcubierre warp bubble bending and blue-shifting the sky around a ship in warp (`ShipWarpQuality`). Ships are kept between sessions in `spacecraft.cfg` (SE's keys), and the one being flown is flown again on the next start. With orbits shown, each flying ship draws its trajectory (the flown one in the selected-orbit colour). The Ships window has SE's Unlimited velocity and Invulnerability cheats (a crash destroys the ship without it; "Cheater!" shows in the HUD). Ship hulls take their design's `Exposure`. The catalog's spacecraft stand where the catalog puts them: the six Apollo lunar modules on the Moon, built from their ship design. SE's aerodynamic models (none, shuttle, arrow; its coefficient table read from the exe) act in atmospheres, with Mach and dynamic pressure on the HUD; attitude holds (prograde to normal, horizon), hold altitude, sync velocity and an autopilot that rotates to, flies to or warps to a target in SE's stages; engine, thruster, hover and warp sounds play by thrust.
- **Scripts.** SpaceEngine `.se` scripts (the main-menu scene and the tutorials) run through `--script` or the Scripts window (`docs/formats/se_script.md`), including `Screenshot` (with `WaitTrigger "ScreenshotComplete"`), `SavePNG`/`SaveJPG`/... image conversion, `Show`/`Hide` of display items, `DisplayMode`, `SetLocale`, `ReloadConstellations` and `PrintVars`.
- **Camera modes and capture.** SE's movement modes (1 Free, 2 Spacecraft, 3 Aircraft, 4 the flown ship; saved in `save.cfg`), continuous PgUp/PgDn zoom; the HD screenshot tool (Shift+F11, tiled mosaics), skybox export (cube faces, DDS cubemap, panorama, 8/16/32-bit) and SE's fish eye, cylindrical and cubemap projections (Settings > Projection).
- **Render scale.** The 3D view renders at SE's `SolidResolution` (capped by `SolidTargetRes`) and is upscaled with SE's filters (nearest, bilinear, bicubic, lanczos; plain, sharp or CAS) and FXAA, and the final frame is sharpened with SE's AMD FidelityFX CAS (`Sharpness`), in Settings > Graphics. Memory caches take SE's `CacheSize*` sizes, `MaxThreads` sizes the worker pool, and `TimingStrategy 1` loads terrain by SE's time budget. `BlackHoleQuality` sets how often a black hole's sky map is redrawn (0 turns the lensing off).
- **Reference tools.** The wiki (object descriptions from the locale database), the star browser and the system chart. The star browser's Search tab (main toolbar "Search", or Shift+F3) is SE's search: it scans up to 10,000 systems within a radius (catalog and procedural, generating each system as a visit would), and its Filter settings pick planets or moons by class, temperature, water, atmosphere, life (organic or exotic, unicellular or multicellular, marine or terrestrial), radius and catalog or procedural systems; the last column counts the matches, lists them on hover and selects them in turn on click. With "Star" as the object it lists the nearest 10,000 stars of a type (black hole, neutron star, white dwarf, brown dwarf, giant, or a spectral class) without generating planets. The procedural star field holds neutron stars and black holes as SE's does (4% and 2% of its white-dwarf slots).
- **Constellations and grids.** SE's constellation lines between the real stars (so their shapes change when you travel), IAU boundaries and names (full or abbreviated), and the equatorial, ecliptic and galactic grids with SE's labels plus a planet's geographic grid; all off by default, with SE's keys and fades.
- **Info panel.** The Selection window's full info adds SE's derived rows: the Hill / influence sphere radius (`a (1 - e) cbrt(m / 3M)` and `a (m / M)^0.4`; a binary's members show their barycentre's), the Roche limit for satellites (1.26 lunar-density radii), orbital resonances with sibling bodies (SE's small-integer period ratios within 1 %, e.g. Io: Europa 2:1) and the life rows (forms, biomes, abiogenesis or panspermia). Generated cluster stars include SE's Wolf-Rayet (WN, WN/C, WC, WO) and carbon / S-type (MS, S, SC, C-R..C-Hd) conversions, with SE's class names.
- **Selection pointer.** SE's: four cyan ticks turning around the selection, outside its disc, and a hollow triangle at the screen's edge pointing to a selection off screen (Settings > "Selection pointer").
- **Procedural galaxies' brightness.** SE has no galaxies at the octree's level 0 (M -32 to -28); the port dropped them, and draws the others (whose 50 kpc radius cap packs their light into too small a disc) at the surface brightness of SE's radius law, as SE does.
- **Star points style.** SE's `StarPointMethod` (Settings > "Star points style"): sprites, sprites with motion blur (SE's default) or with motion and rotation blur. The blur stretches each star and point-like galaxy along its path on screen over the last 1/60 s of camera motion (and turning), spreading its light along the trail; `MotionBlurLength` scales it.
- **Redshift.** Distant galaxies are tinted as by cosmological redshift: a 5500 K spectrum shifted by z = d / (c/H0), at constant brightness (about 10% redder at 1 Gpc, orange by 3 Gpc).
- **Lens flares.** SpaceEngine's flare, ghost and pseudo-flare presets from `Flares.pak` on the suns of the current system, faded by the sun's magnitude and scaled by the frame's brightness at the sun as SE does, so whatever covers or reddens the sun (a planet, a mountain, the horizon at sunset) dims and tints its flare; the pseudo flare has SE's lens dirt and starburst (`docs/formats/lens_flares.md`).
- **Graphics presets.** Potato to Ultra, read from the install's `config/preset-*.cfg` and kept whole: MSAA, FXAA, anisotropic filtering, landscape LOD, terrain tiles per frame, volumetric quality, star points style and black-hole warp precision, in Settings > Graphics with the lens flare options. The first run picks the preset from the video memory, as SE does.
- **Exposure.** Faint background light (galaxies, nebulae, stars) recedes while a sunlit body or a star fills the view, as the eye adapts. SE's photo modes (B here): HDR, Manual and Auto, whose auto exposure meters SE's luminance histogram; Manual and Auto keep the HDR brightness calibrations (SE's `real*` values are not used). Tone mapping: SE's 11 tone shaders (Settings > Graphics), default AgX.
- **Interface and audio.** Navigation and interface as listed under Controls, SE's fonts (Noto Sans with its Medium bold, CJK and DejaVu fallbacks), object types named as SE names them ("Yellow dwarf (G2V)", "Planet (Temperate marine terra with life)"), the loading screen ("Sandbox Engine Loading..." on black until the first view is loaded), the solar system browser, interface click sounds, and the soundtrack with SE's cross-fades. The window opens maximized and remembers its size, fullscreen mode and VSync. Settings persist in `%APPDATA%/SpaceEngine-rs/settings.cfg` in SE's key names (with post-effects, export and the whole graphics preset), the camera, binding, selection and time are saved on exit to `save.cfg` and restored at start, and places you save from the Places window go to `places.cfg` next to it, in SpaceEngine's own format; scripted runs (`--run`, `--script`, `--screenshot`) leave both untouched. Every start rewrites the log `se.log` there, in SE's format (`[MT]` thread stamps, optional time stamps, catalog messages by SE's `CatalogLogLevel` / `CsvLogLevel`).

### Not yet implemented

- **Gameplay.** Everything beyond flying one ship: docking, fuel and heat, aircraft control surfaces, damage.

See `docs/` for the reverse-engineering notes: formats, frames, procedural rules,
atmosphere and galaxy specifications.
