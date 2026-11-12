# SpaceEngine Rust/Bevy port, version 2

**This folder is version 2** (`C:\Users\liamw\Desktop\SpaceEngine Rust Port Version 2`), copied on
2026-11-02 from version 1 (`C:\Users\liamw\Desktop\Space Engine Recomp`), which stays as it is.
Version 2 is where better visuals, add-ons and the like go. Work here, not in the version 1
folder. The test scripts (`.audit/tools`, `.optimize/bench`) point at this folder; the older
briefs and reports in `.audit`, `.bugfinder` and `.optimize` still name the version 1 folder, as
history. Version 2 keeps the user's settings, log, saved session, places and ships in its own
`%APPDATA%\SpaceEngine-rs-v2` folder (`settings.rs` `USER_DIR`; the atmosphere cache too). On its
first run it copied version 1's `%APPDATA%\SpaceEngine-rs` there (done 2026-11-02, by a test run);
it never writes version 1's folder. Screenshots still go to the shared `Pictures\SpaceEngine-rs`.
Test runs write `se-test.log` there (since 2026-11-04; the user's own `se.log` is kept for looking
into what they saw), and skip the settings and session.

A private rebuild of SpaceEngine 0.991 in Rust (Bevy 0.20), drawn with Vulkan. It reads the
game's own files from the SpaceEngine install at run time; nothing from the game is copied here.
README.md explains the features, controls and command-line options; `docs/progress-archive.md` holds
finished work's detailed notes moved out of PROGRESS.md; `docs/` holds the notes on
SE's file formats and reverse engineering; `docs/missing_features.md` lists what still differs
from SE. `.bugfinder/REPORT.md` is the 2026-11-01 bug hunt: what was found, fixed and left.
`docs/ksp-clouds-notes.md` is how blackrack's volumetric clouds for Kerbal Space Program work (studied
for the clouds plan); what it was learned from (his public C#, the user's download's shaders exported
as GLSL, his wiki) is in `re/ksp_clouds/`, which stays out of git: someone else's code. Take methods
from it, never its code, shaders or textures.
`docs/spec-mods-viltrumite-powers.md` is the plan for the Mods menu and its first mod, a playable Omni-Man (the
user's interview of 2026-11-06 is in its Decisions log; its research tools are in `re/viltrumite/tools/research`, out of
git, and `re/` must be kept).
`docs/spec-vegetation-civilizations.md` is the plan for vegetation and intelligent civilizations (the
user's interview of 2026-11-04 is in its Decisions table). Its reference is the game Megaton Rainfall
(the user's GOG copy in `C:\Users\liamw\Downloads\Megaton Rainfall`): how it draws plants and cities is in
`docs/megaton-rainfall-vegetation.md` and `docs/megaton-rainfall-cities.md`; its text files and shaders
are unpacked in `re/megaton_rainfall/extract` and the scripts that read its formats are in
`re/megaton_rainfall/tools` (both out of git). Its methods are learned, not its code; its assets (trees,
grass, facade kit, window textures, roads, props, cars) the user chose to reuse. Since 2026-11-05 the
ones the port uses are copied into the project (`crates/se_engine/assets/mr`, the user's decision: the
port must work without the game installed). Since 2026-11-09 the user ships that copy with the port
(built into the exe), saying the files are free to use.

**The sandbox (the user's feature of 2026-11-11):** the game starts on Earth's ground in first person, with Garry's Mod's
movement, controls, weapons and sounds, a spawn menu on Q held down (props, weapons, the mods to play as), props with real
physics under the planet's own gravity, and V switching the port's free flight on and off. It is `crates/se_engine/src/sandbox/`
(`mod.rs` says what each file is) and `crates/se_core/src/gmod.rs` (the reader of Garry's Mod's files). The references it was
built from: Valve's Source SDK 2013 (`gamemovement.cpp`, the weapons' `.cpp` files) and Garry's Mod's own Lua and scripts in
the user's install (`D:\SteamLibrary\steamapps\common\GarrysMod`: `garrysmod/gamemodes/sandbox`, `cfg/config_default.cfg`,
`settings/spawnlist_default`), read for their numbers and rules, never copied.

## Build and play

- Build: `cargo build --release` (always release). The user plays from
  `target\release\spaceengine-rs.exe`, so rebuild release after every change.
- Tests: `cargo test -p se_core` and `cargo test -p se_engine --release`.
- SpaceEngine install (read only): `D:\SteamLibrary\steamapps\common\SpaceEngine`.
- No .bat launchers yet.
- What a launch does (since 2026-11-11): it starts on Earth's ground in first person (the sandbox). `SE_SANDBOX=0` gives the
  port as it was (free flight at Earth, the saved session restored); test runs (`--run`, `--screenshot`, `--script`) and
  `--start NAME` keep the old start unless `SE_SANDBOX=1`. The sandbox's props' physics is the `rapier3d` crate (0.36, its own
  world, no Bevy plugin), and Bevy's `wav` feature is on for Half-Life 2's sounds. Since 2026-11-12 the main menu comes
  after the loading screen (the sandbox's `main_menu.rs`); test runs skip it unless `SE_MENU=1`, `SE_MENU=0` skips it in a
  normal launch, and there is none with `SE_SANDBOX=0`.
- What players get (since 1.0.1, 2026-11-09): the exe alone. `crates/se_engine/build.rs` builds every file under
  `assets/shaders`, `textures`, `models`, `mr` and `mods` into it (`embedded_assets.rs`); on a player's first start
  (and when an update changes them) they're unpacked to `%LOCALAPPDATA%\SpaceEngine-rs-v2\assets` (about 115 MB) and
  read from there. Order (`config::asset_dir`, found once): an `assets` folder beside the exe with `shaders` in it, then
  the source tree but ONLY for an exe inside the project folder (`config::source_assets`: `target\release`, tests, bench
  copies in `.optimize/scratch`, so shader edits need no rebuild there), then the unpacked copy. Never read
  `CARGO_MANIFEST_DIR` at run time outside tests: go through `config::asset_file` / `asset_roots` (v1.0.0 did, and
  everyone else's universe was black). If nothing is found the port stops with a message (`ownership::check_assets`).
  The Megaton Rainfall install default (`DEFAULT_MR_PATH`) is used only by an exe inside the project. A shader edit
  makes `cargo build` recompile se_engine (the build script embeds it). Test a release the way a player gets it: the
  exe alone in a folder outside the project, with `LOCALAPPDATA` and `APPDATA` set to empty scratch folders (a
  new PC: it must unpack, find SE through Steam and log its files' folder there). `bash .audit/tools/package_release.sh
  VERSION OUT_FOLDER` makes the zip (Python's zipfile: PowerShell's Compress-Archive writes backslashes in the paths).
- The ownership check (`ownership.rs`, the user's 2026-11-07: friends may play it only with their own SpaceEngine): before
  anything reads SE's files, `main` calls `ownership::check`, which sets `cfg.se_path` to the verified install. First run:
  the copy Steam lists is kept without asking if it passes (since 2026-11-09), else a Windows dialog asks for SpaceEngine.exe; it passes only if it is SE's program (version resource), in a Steam library
  beside `data`, with `appmanifest_314650.acf` naming its folder (StateFlags 4), the library in Steam's
  `libraryfolders.vdf` with 314650 in it, and the manifest's `LastOwner` in Steam's `config/loginusers.vdf`. The choice is
  kept in `ownership.cfg` in the user folder and checked again every start. Test runs open no dialog (they check the copy
  Steam lists, writing nothing); `--se-path` / `SE_PATH` must pass the same check. It reads Steam's files only, never runs
  or changes anything. The loading screen is only "Sandbox Engine Loading..." on black (`splash.rs`, the user's choice):
  don't bring SE's splash art back.

## Rules from the user

- Vulkan only. VR and SE's editors are out of scope.
- SpaceEngine's assets are read from its install, never copied into the project. Megaton Rainfall's
  are the exception (the user, 2026-11-05: the port must not rely on that game): the files the port
  uses are copied into `crates/se_engine/assets/mr` and read from there. Since 2026-11-09 the user ships
  them with the port, built into the exe (the user: they are free to use). The Viltrumite
  powers mod is the second exception (the user, 2026-11-06): the Omni-Man model and the parts of a GTA 5
  Superman mod it uses are converted once, by a tool outside the game, into `crates/se_engine/assets/mods/viltrumite`,
  shipped the same way since 2026-11-09; the game has no GTA format readers, and that mod's compiled code and GTA V's encrypted
  archives are never opened. Any other game's assets stay in their installs.
- Garry's Mod's files (the user, 2026-11-11: "keep the original installation unchanged", "use placeholders where needed") are
  read from its install while the game runs (found through Steam, app 4000, or `GMOD_PATH`), never copied into the project and
  never changed there: the third game after SpaceEngine and Megaton Rainfall, and like SpaceEngine's they stay in the install.
  Without Garry's Mod the sandbox must still work (plain props, plain sounds: `sandbox/assets.rs`).
- V switches the free flight on and off and does nothing else (the user, 2026-11-11): photo mode moved to B. Q held is the
  spawn menu while on foot or playing a mod (in free flight Q is still the roll key, as "keep the existing flight controls" says).
- When SpaceEngine itself needs running (to compare with it), a Sonnet agent (high effort) drives
  SE and takes the screenshots.
- Commit each piece of work once it builds, passes the tests and is checked; never push. Commit
  messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Keep each file's line endings (many files are CRLF): don't use `sed -i` on them.
- Don't defeat DRM. Never delete the user's own data (settings, saved sessions, places,
  screenshots, the game installs); scratch, build and test files may go.

## Gotchas

- The sandbox (`sandbox/`; README's "The sandbox" has the keys). Three modes (`sandbox::Mode`): `Walk` (the player owns the
  camera and the keys: `ModControl::walker`, set every frame, gates the free camera's mouse, wheel and steering keys in
  `input_controls`, skips `travel_and_move`, hides the panels and the selection pointer), `Fly` (the port's free flight,
  untouched) and `Hero` (a mod plays; `ModControl::spawn_menu` frees the mouse for the Q menu). `read_input` (PreUpdate, before
  `mods::snapshot_input`) reads his keys and clears their presses so nothing else sees them (Space pauses the clock, 1-4 change
  the flight mode...): a new key of his goes in its `HIS` list.
  - The camera: `sandbox_step` runs in `UniverseSet::Camera` after `travel_and_move` and BEFORE `hero_move`, writes the camera at
    his eyes and binds it to the body as of this frame (`bind_camera`). The body moves 500 m a frame round the Sun: whenever the
    sandbox stops writing the camera (V, a mod taking over) it must write it once more in that same frame and leave it bound, or
    whatever takes over starts 500 m off (it did: the free camera jumped 230 m into the air on V). Leaving first person is
    therefore done at the END of `sandbox_step`. While he waits to be set down (`Pending`) the camera is held on the body too.
    `FlyCamera::eye_floor_m` (set every walking frame) lets his eyes below SE's 1.5 m floor in `keep_camera_above_ground`.
  - Units and frames: `movement.rs` is in Source units (`UNIT_M` 0.01905 m), x east, y north, z up, in `world::LocalFrame` (east,
    north, up at an anchor on the body, turning with it). The anchor follows the player (`REBASE_M`), because rapier is `f32`.
    Garry's Mod's models are x forward, y left, z up; its weapon world models point along +y (the gravity gun's along -y).
  - The ground is a height field sampled from `ship::ground_on` (the terrain's loaded heights) 60 m round the anchor, remade
    when he walks off, and when the terrain under it changes: a coarse terrain node can stand 150 m off the real ground, and
    finer ones arrive over the first half minute. `replace_ground` (and the player's share in `step_world`) keeps everything's
    height over the ground through a change, or it is all dropped or buried (he fell through the planet). `unstick` lifts him out
    of the surface. Props off the field are frozen until it covers them again.
  - rapier: rays and sweeps go by a tree that only a step updates: after adding, removing or moving anything by hand call
    `SandboxWorld::refresh` (`detect_collisions`), or the next query misses it. In `cast_shape`'s answer `normal1` and `witness1`
    are the collider's, in world space. The player's sweeps go by collision groups: a collider he must meet has `G_PLAYER` in
    its filter.
  - Light: props and the weapon are lit by `ground_light.wesl` through the hero's light pass run a second time for the player's
    place (`viltrumite/draw.rs` `LightPass`, `LightProbe`; `sandbox/light.rs`; `sandbox_prop.wesl`): change `hero_light.wesl`'s
    bindings and both users follow. `PropMaterial` is never changed after it is made. Effects (muzzle flash, dust, the physics
    gun's beam) and the HUD are egui shapes (`weapons::draw_hud`).
  - Garry's Mod's files: `se_core/gmod.rs` (VPK archives, `.mdl`/`.vvd`/`.vtx` models, `.vtf` textures, `.phy` collision pieces,
    WAV sounds incl. ADPCM, spawn lists). Its tests read the install and skip without it. A tenth of the models keep their
    collision pieces in another pose's frame: `assets::collision_shape` falls back to the mesh's hull when they don't fit.
    Half-Life 2's scripts are loose files under `sourceengine/`: `GmodInstall::loose` looks there after `garrysmod/`.
  - Testing: `SE_SANDBOX=1` in a `runport.sh` run; `sbx state|props|solids|arsenal|look YAW PITCH|spawn NAME|undo|clear|menu on|off|mods|weapons|
    weapon N|walk [LAT LON [HEADING]]|fly` (`sandbox::dev_command`; `look`'s yaw is 0 east, 90 north, its pitch up); keys with `hero keys w+shift 2` (also `lmb`, `rmb`, `v`,
    `space`, `ctrl`, `1`..`6`: a new `hero keys` lets the old ones go and presses the new in the same frame, so `lmb` then
    `lmb+rmb` keeps the left button down); `uiclick X Y` clicks the Q menu; `SE_SANDBOX_SOUNDS=1` logs each sound as it starts to
    play (a test can't listen); `SE_SANDBOX_SPOT=N` the starting place; `GMOD_PATH=C:/nonexistent` the no-Garry's-Mod path.
    `surface LAT LON ALT_KM`'s height is over the ellipsoid, not the ground (the Serengeti stands 1.7 km over it). A new starting
    place goes in `SPOTS` and must be checked to be dry land (the "set down" log line says "(the sea)").
  - Solid buildings and trees (`sandbox/solids.rs`, the user's 2026-11-11): every walking frame the walker asks the scatter's
    probe (`ProbeAsk`, the Viltrumite mod's: `scatter/probe.rs`) for what stands within 45 m, and trees within 35 m
    (`ProbeCapsule::trees_m`, the shader's `body.street.w`; `probe_tree` in the cull lists them as records of kind 4, which
    the hero's `Solid::of` skips). `Solids::sync` makes rapier colliders of the list (group `G_SOLID`): a building's slices
    from `viltrumite/solid.rs` (boxes, hulls for turned ones), a tree's trunk as a cylinder from `ScatterLibrary::trunks`
    (the bark's radius at chest height). The list is 3-5 frames old; what would hold him inside waits until he is out; a
    rebase clears them (`solids_cleared`) and they are made again. Nothing is solid that isn't drawn. `sbx solids` lists the
    nearest with the yaw to walk at them; a wall or trunk met at an angle slides him along it (Source's rule), so test by
    walking straight at one. Checked in New York: a wall stops him, a roof holds him (V, `surface`, `agl 22`, `sbx walk`),
    a shot at a wall plays concrete's sound.
  - The scatter places a cell once (`gpu.rs` `placed` keys). After a jump of hundreds of km (`sbx walk LAT LON`, a Go To, a
    place) the cells in range were placed against the old map patch (`scatter/earth.rs`, 600 km wide) and stayed empty: New
    York stood without its near buildings and trees. `Scatter::replace` counts the patches that reach the camera again
    (`EarthVeg::covers`) and is part of the slots' key, so everything is placed anew then. Keep that when touching either.
  - The weapons' view models (`sandbox/viewmodel.rs`, `sandbox_viewmodel.wesl`, `se_core/gmod.rs` `skeleton`): Half-Life 2's
    `v_*.mdl` with bones, sequences, events, the muzzle attachment and bone weights, skinned by a palette buffer written in
    place (as the hero's), drawn with their own field of view (`FOV_RATIO`: 54 against 75 degrees, x and y scaled in view
    space) in the nearest depth slice. Placement is the SDK's `CalcViewModelView`: the eyes, the view WITHOUT its punch, the
    bob and the lag (`Sway`). Model space is Source's (x forward, y left, z up). `weapons.rs` is the SDK's weapon code rule by
    rule (deploy times from the sequences' own lengths, the pistol's accuracy penalty, the SMG's kick, the shotgun's reload
    states); sequence names differ per model (`WeaponDef::sequences`, checked against the files by a test). The gravity gun's
    prongs are its autoplay sequence `prong_blend` (two animations and a weight list: `GmodSequence::blend_to`, `weights`)
    laid over whatever plays by `ViewModel::open`. The crowbar's miss swing really does leave the screen for a third of a
    second. `sbx arsenal` logs what is in hand and playing. Without Garry's Mod the stand-in world models are used.
  - Sounds (`sandbox/soundscript.rs`, `audio.rs`): a cue names one of the game's sounds (`Cue::named("Weapon_SMG1.Single")`)
    and the install's scripts (`scripts/game_sounds_manifest.txt`, `surfaceproperties_manifest.txt`) say its waves, volume,
    pitch, level and channel. A new name goes in `audio::NAMED`: a test checks every one, and every surface's five sounds,
    against the install. Rules from the SDK that must stay: a surface starts as `default` (dirt names no soft impact); a
    step's volume is the code's, by `gamematerial` and speed, not the script's; a sound on a channel other than auto and
    static cuts the last one there (`ChannelVoice`: without it the SMG's shots piled up to the ceiling); each wave of a set
    plays before any plays again (`SoundScripts::pick`); step times are Garry's Mod's (350 ms, 250 running). Placed sounds
    are spatial voices half a metre from the ears in their direction, their loudness by `distance_gain` (Source's
    `SND_GetGainFromMult`). **rodio's `Spatial` makes the ear FURTHER from a sound the louder one, so `spawn_ears` swaps the
    ears' offsets on purpose** (measured: don't "fix" it). The ground is concrete among buildings, grass or dirt elsewhere.
  - Measuring sound (no test can listen, but the speakers can be recorded): `.audit/tools/loopback` (its `Cargo.toml` says
    how to build it) records the default output device to a WAV and `analyze.py` lists each sound's time, peak and
    left/right balance. Run the game WITHOUT `timing frame` (sounds would overlap) with `SE_SANDBOX_SOUNDS=1` and line the
    events up with the log's times (the log is UTC). The user hears the run. Measured 2026-11-11 at the default volume
    (`SoundVolume 30`): pistol peak 0.43, shotgun 0.67, an SMG burst 0.79, walking steps 0.04-0.07, nothing clipped; a
    barrel dropped on his right is 5.7 dB louder on the right.
  - The street lamps light the props and the weapon at night as they do the hero (`LightProbe`'s town share, `draw.rs`
    `town_light`; the term is in `sandbox_prop.wesl` and `sandbox_viewmodel.wesl` as in `hero.wesl`).
  - The main menu (`sandbox/main_menu.rs`, the user's 2026-11-12; layout from Garry's Mod's `html/`, nothing copied): an egui
    Area at `Order::Background`, so the Settings window (Options: `hud.show_settings`, Middle) draws over it, and the loading
    screen (Foreground) covers it until it has faded (`Splash::covering`: no click before). egui paints layers that aren't
    areas (`ctx.layer_painter`) after the areas of their order, so they land on it: each must be gated on
    `ModControl::main_menu` (the walker's HUD, his readout, the selection pointer, the centre messages, orbits, labels and
    markers, constellations, grids and waypoints are; test with `SE_OVERLAY=orbits,labels=2,markers,constellations,grids`:
    test runs have them off). The panels go by `panels_hidden`, which includes it; the port's other windows (Find, Places,
    Help, the star browser, Ships, the scripts' toolbars, the ship's HUD) by `hud.hidden`, which the menu turns on while up
    and gives back after (`saved_hidden`); the Settings window obeys neither, so Options works. While it is up `menu_input` (PreUpdate, after `InputSystems` and
    `viltrumite::dev_keys`, before `sandbox::read_input` and `mods::snapshot_input`) resets every key and button and zeroes the
    mouse's motion and wheel; Esc's held state is kept, or the key's auto-repeat presses it again. `ModControl::main_menu`
    (in `menus_open` and `panels_hidden`) frees the cursor and lets the pointer reach egui in `sandbox::read_input` /
    `strip_egui_input` and `mods::snapshot_input`, without surrendering a text field's focus (Options). Esc: a flown ship's
    first (ship.rs), else the menu (Options, then the Mods page, then closed). Mods is the Q menu's own rows (`menu::mod_rows`).
    Play once playing goes back on Earth through `restart_on_earth` (the mod off first, then `put_on_earth`, the launch's own
    spot choice). Pictures: `screenshot::folder()` and its `screenshots-hd` (never `export`), else Garry's Mod's
    `garrysmod/backgrounds`, decoded on worker threads (the `image` crate's jpeg and png), two textures at most. Tests: `SE_MENU=1`,
    `menu on|off|state|mods|main` (`state` logs each entry's middle for `uiclick`), `hero keys esc 0.2`; `main menu: cursor`
    lines log the grab for runs with a real mouse.
- Mods menu (`mods.rs`; the plan is `docs/spec-mods-viltrumite-powers.md`, its Build log says what each step did): a
  mod is a folder under `assets/mods` with a `mod.cfg` (`Name`, `Description`, `Code`, repeated `Need "relative path"`).
  The scan and `mod_file` look beside the exe first, then in the source tree, per file (as orbital.rs's textures). List
  in `Need` only files that exist: one missing greys the row. A new mod's `Code` goes in `KNOWN_CODES`. Every way off goes
  through `ModControl::off`; `off_for_pilot_or_script` runs after `ship_input`, `move_mode_keys` (key 4) and `input_controls` and before `run_script`,
  so the mod is off first. With the mod off only run conditions run; keep it so. `SE_MODS=0` hides it all. Test a grey row
  with a copy of the exe beside `assets/shaders` and `assets/mods/NAME/mod.cfg` (`SE_UI_OPEN=mods` or `mods window`), and
  wait 1500 frames before judging a picture: earlier ones can be black, even the panels. A pale Earth ground with the rocks
  still brown (`refs.sh` used to catch it at 1200 frames) is the ground half loaded, not a bug: it is gone by 3000.
- Viltrumite's files (`assets/mods/viltrumite`, 52 MB) are written only by `python -I re/viltrumite/tools/convert.py` (out of
  git, its Blender and Mixamo routes too; `--check-guards` shows what it refuses to open, `sheet.py DIR OUT.png` draws him posed from the files alone:
  look at it after any change). Never edit them by hand; re-running replaces everything (Ogg files differ byte for byte each run, the rest doesn't).
  `se_core/src/hero_asset.rs` is the format (`omniman.sevm`, `clips.sevc`) and convert.py's `write_sevm`/`write_sevc` must change with it. Model space:
  metres, Z up, +Y the way a clip faces, +X his right hand, origin at his pelvis; the bind pose faces -Y only because `SKEL_ROOT`'s rest rotation is 180 degrees about
  Z, and a full clip's root track undoes that. Add a file: list it in convert.py, which writes the `Need` lines in `mod.cfg`. A sound goes through
  `mods::check_sound` (the guard, and the decoder's own panic caught) before anything plays it. A looping Ogg's length must be a whole number of 2048 samples
  (`make_loop`): otherwise the encoder pads the end with silence and the loop gaps. The Mixamo clips: FBX files in `re/viltrumite/mixamo`, re-run, they replace the stand-ins
  (`Clip::stand_in`); the retarget needs the file's rest pose to be the T-pose and warns if not; `make_test_fbx.py` and `--check-test-fbx` test the route without them.
  Clips that aren't plain loops: the 14 lean clips (`CLIP_LEAN`) are one-way ramps, sampled at `lean * duration` and never looped or mirrored (a held lean would come and go); `fall` settles and
  goes on as `fall_loop`; walks, runs and sprints are never mirrored (`close_loop`'s `mirror`: the feet would slide backwards). Normalise quaternions before comparing them: their length errors read as a degree of turn.
  First enable looks inside every `Need` by its extension (`mods::check_files`: `parse_dds`, `check_sehm`, `check_sound`), so a new kind of file gets its check there; a refusal greys the row
  (`ModRegistry::mark`: `Missing` if the file has gone, `Bad` if it's wrong).
- Drawing the hero (`viltrumite/draw.rs`, `anim.rs`, `hero.wesl`, `hero_light.wesl`, `hero_frame.wesl`; spec F13): his light is `ground_light.wesl` `point_light_at`, the same
  function the scatter's objects use (change the ground's lighting there and all follow), run once a frame by a compute pass; his draws read the result. Near a body whose light record is fresh (`Terrain::body_light_fresh`; an empty `drawn_nodes()` doesn't say so: under a giant's radius the list is empty and the record fresh), under
  its atmosphere's top, `HeroFrame` copies the body's light record and atmosphere (as scatter/gpu.rs); otherwise `free_light` fills the same record on the CPU, further lights in `free_*` taken with
  his normals. `planet_intensities` can return fewer values than lights (it ends at the first too faint): never index it blindly. Palette: one `ShaderBuffer` written in place, never a material; meshes at identity,
  camera-relative, `update_hero` after `update_node_transforms` and before `CameraUpdateSystems` (the near plane too). His pipelines queue on first enable under a palette of zeros (`HERO_PIPELINES`: add any new pipeline of
  his there, and its shader file to `pipeline_watch::hero_pipeline`). Skinned normals: green of the normal maps points down the image, as +v: `b = cross(t, n) * w` (it was found by the curl of the maps, not by eye). A cape joint's rotation must include the chest's
  (`hang_cape`), or the normals stay on the cloth's far side and it looks pale. The main world has no vertices after first enable (`HeroAssets.model.vertices` is emptied once the meshes are made). `hero clip NAME|auto`, `light`, `state` help pictures (a test's `mods on` needs a `wait` before them). Heights that compare with an atmosphere or a cloud layer (the fade in `hero_light.wesl`, `update_hero`'s limit) are the body's sphere frame's: the radial distance reads up to flattening x radius too low, 21 km at Earth's poles, 6,000 km at Saturn's. The street lamps on him are `RoadCtx::town_at` (`Terrain::town_ctx`, level 2 and up: the ground's own density, so none over the sea or in a village), on a worker thread because a map body's reads a lights tile. `update_hero` runs before `capture::follow_main` (the capture views copy his near plane, or he is cut from hi-res shots). `free_light` takes the stars through `star_tint(.., true)`: `sun_sprite_tint`'s sphere hides the star from a point above a flattened giant's pole. His white suit reads 15-25 times brighter than Earth's forests: that is albedo (conifers are about 4 %, and the trees take the map's dark colour), not a light bug. Pixel comparisons that are bit-exact: `.optimize/bench/pixel_scenes.sh`, `freeze_clocks.py`, `pixel_cmp.py`.
  `town_night` has two states whatever the exe (2026-11-07: three runs each of the baseline and of step 9a's build gave the same two
  pictures, bit for bit, in no order): two runs differing by exactly 13.447 % of pixels (0.706 % by more than 2 levels, a fine grain on
  the facades and windows' edges) is that flake, not a change. Run it three times or more and look for a pair that matches bit for bit.
- The hero on the ground (`viltrumite/ground.rs`, `hero.rs`; spec F4, F6): walking is `ground::walk_step` on `Frame::Attached`, never `Frame::advance`.
  `support_height` is the ONE height walking, the feet, the shadow and the camera's floor use (step 9 adds a building slice's top there). The capsule's middle stays
  `STAND_M` over it, followed exactly; what's drawn is `post_pose`'s lift: the clip's `ground_z`, `Hero.ease` (a finer node moved the ground: eased, and the camera goes with
  it) and the root's drop to the lower foot (at most 0.25 m). Change a state between ballistic and not only before `Frame::carry` or after `advance`, as `carry`'s comment says: walking off something (`WalkEvent::Ledge`) changes it between them, so `hero_move` skips that frame's `advance` (`fall_starts`), or he hops by the ground's turn, 7.75 m at Earth's equator. Slopes: walk to 50 degrees, slide to 70, a wall above: `walk_step` turns a wall into a Ledge and `advance_attached` lands him on one only if he hits it at 35 m/s or more; otherwise he goes on down it at free fall (on a height field a landing and a fall would go on for ever: straight down from the ground is under the ground). Hits go by the ground's normal at the hit, flying and falling (`Outcome::flown_into`, `contact`), radial only for what the slide keeps; `ground::flight_lands` says when flying into it is a landing (Ctrl, or 12 m/s on what he can come down on; a giant or a star only stops him when asked), `Outcome::at` where he hit (a flight's slide has already gone on from there). `flown_into` only where a level exists (F2). `keep_camera_above_ground`'s push is told apart from a jump of the camera by `FlyCamera::floor_push` (a finer node rising past the camera's floor once dropped him out of any state into Hover).
  `Footing` says what he'd come down on: only `Land` gives Grounded; `Sea` is `hover_over_sea` (`keys_up_first`: or running off a beach launches him), `Nothing` a plain Hover.
  Turn clips turn the root in the clip (90 degrees): his body holds still, and when one ends the Director hands the turn to `hero.rot` and `Sampler::rebase_yaw` turns the frozen
  pose back, or the crossfade spins him. `hero feet FILE` logs his soles against the ground a frame (`hero trace` still logs the rest); `hero dist M`, `hold on`, `soil off`,
  `shadow off` are for pictures and measuring. In a `--run`, `hero trace` and `hero feet` paths mustn't hold a space: use a relative path (`trace` cuts its body name at the
  first space: a path with one wrote a stray file named after its first word).
- The soil hold-down (`planet.wesl`'s vertex shader, the `cloud_sh.hero` it reads): a line from where his soles' ray lands to where his head's does (`HeroSoil`, `draw.rs` `fall_over`), so his feet and his shadow's plane, which go by the CPU's heights, lie on what's drawn, and `w` how much it holds, by his height with the shadow's strength: no step at any height. `CloudShadow.hero` is w 0 with the mod off, so the ground is bit-identical; it and `hero_end` must stay the
  LAST fields of `CloudShadow` and be in `ground_light.wesl`, `clouds.wesl` and `volumetric_clouds.rs`. Written from `HeroSoil`, only when it changes.
- His shadow (`draw.rs` `shadow_plane`, `hero.wesl` `vertex_shadow`, `fragment_shadow`): the same meshes in a Multiply material, without the cutouts. Depth compare Greater with
  `frag_depth` from the pixel's ray on the plane through `view.main_pass_viewport` (never `frag_coord_to_ndc`, wrong under the render scale): every triangle of a pixel writes
  the same bits, so the first wins and nothing double-darkens. Clouds and eclipses scale the sky light with the star's, so the factor alone doesn't fade under them: `hero_light.wesl` writes their
  share of the direct light in `HeroLight.sun.w`. Its `depth_bias` of 2e10 is the transparent phase's sort: it must draw after the sea (1e10), which writes depth: the plane is 3 cm above the water, and drawn first it kept the sea out of his whole silhouette, waves and glint with it. The plane contains the line from his soles' ray's landing to his head's, by `ray_landing` over the real heights (`fall_over`), and every vertex ends on it (what dips under goes straight up to it, or its triangles lie on his legs and darken them). A
  material with two vertex entry points must name its entry point in `specialize` (Bevy leaves it to the module's single one, and wgpu quits the game otherwise).
- His cape (`cape.rs`): points in model space, every step ending with each link at its exact length from the chest, so it can't explode; everything that moves it is clamped
  in `hero_move` while it's still f64 (`cape_in`). A really fixed step with what's left over drawn between the last two (`Cape::left`): a position solve loses speed by the step, so a step that changes with the frame changes the swing. It runs on real seconds: `hero.rot`'s `was_rot`, `last_vel` and `felt` are taken or turned after `carry`'s turn, or at x100,000 the planet's turn blows it out sideways. `CAPE_FOLLOW` makes it turn with his chest as well as lag: without it a turn on the spot left it in front of him. Bones the
  capsules name must exist, or a capsule goes unused silently.
- The hero's motion (`viltrumite/frames.rs`, `hero.rs`, `camera.rs`; spec F4, F5, F7, F8). `Frame::Attached` (below `min(0.5 R, 300 km)` over the body's highest ground, leaving at 1.2x) is in the body's turning frame; `Frame::Free` is the
  frame the ship rests in (`frame_terms`: the body's motion to 50 radii, its turn to one radius of height), with `vel` relative to it, carried by the body's REAL change this frame, never its velocity: so `sim.jd`'s rounding, x1000 and a date jump can't move him against it. Falling
  he's inertial in both, and only what the game clock did beyond his own turns him (`carry`'s `turn * R(-w dt)`). `carry` and `advance` of one frame belong together: change `state` BEFORE `carry` (a state change after it was a 7.7 m hop against the ground, 210 m on Jupiter) and never apply the turn back at the end of a step (he'd jump by `w x r dt` at each handover). Velocity and spin come from `smooth_velocity`, `smooth_spin` over a
  minute, and the turn's quaternion must be taken with w over 0 (the orientation's sign flips between dates: `spin_at`, used as it was, read 6 rad/s for one frame and threw him 677 km). Proven with `hero trace FILE [BODY]` (a line a frame): step minus speed x dt under a mm across handovers on Earth at x1 and x1000 and Jupiter, flying and falling.
  Input: `mods::snapshot_input` (PreUpdate) reads `ModInput`, then `clear_just_pressed` for his keys, never `reset`; `input_controls` is gated branch by branch, not whole (the spec's F8 table); `FlightHold`, `sync_hold` untouched. The cursor is the snapshot's: into `Locked` only through a frame of `None` that also moves it to the window's middle (locked where it was, a click could land on another window: it did, in a test). Tests can't grab unless `SE_HERO_GRAB=1` (`settings::scripted`), which also makes focus real;
  `hero keys w+shift 3` presses real keys into `ButtonInput`, so every reader sees what a keyboard gives; use `timing frame` so `wait` means seconds (fps goes from 60 to 170 by the scene). Panels are gated in `draw_hud` only (`ModControl::panels_hidden`: not while he's loading, the row says Loading), the Mods window and the Ships button go with them. While he plays the snapshot takes the focus from any text field every frame and Find asks for it only with the panels up, or the keys go dead under a locked cursor. He owns `cam.pos` and `cam.rot`'s use while on: a Go To moves the camera and he's worked out from it; anything else moving the camera over 50 m (`TELEPORT_M`: Places, `surface`) carries him. `travel_and_move` returns early only once he's placed (`Hero.placed`) and nothing travels: until then the camera's binding carries it, or he appears a frame of the body's motion off (589 km at x1000); a Go To's first frame goes with `Hero.carrier` for the same reason. Only WASD cancels a Go To: Shift and Ctrl are held down to start Shift+G and Ctrl+G. A Go To into a body's centre ends at its ground. `camera::level_roll` must take the quaternion with w over 0 (a level view of the other sign read as a whole turn of roll: 85 degrees in a test at x1000). A giant's air counts from its radius (`air_height_on`), never the camera's floor under it: drag, the cap, the readout and steps 6, 7 go by it. The cap's distance comes from his place against this frame's bodies, never `cam.nearest_surface` (last frame's camera, hundreds of km off at x1000). `advance_attached` traces a falling step from where he stood against the ground (`from`, not the point `carry` turned back to) and lets a flown one slide its whole length. Step 5 owns the ground: `Outcome::contact` today only stops him in Hover.
- The hero's controls are the user's own Viltrumite mod's (the user, 2026-11-07; spec D16; `viltrumite/controls.rs`, the end of
  `tune.rs`; its source is `C:\Users\liamw\Desktop\Viltrumite\src`, read-only reference): Space taps (take off, drop) or charges
  (`SpaceHold`: a launch, a boost stack past the barrier), W/S twice for fast flight and out, W the speed ramp in fast flight (the old
  Shift ramp: D6's light speed), Ctrl a dive, Ctrl+Space the super jump, a punch combo and charged punch on the left button. Don't go
  back to F8's keys. Fast flight skims the ground (only a dive or slow flight lands); stack and super-run booms are effects outside
  F9's barrier (C15's silence holds). The camera's shake and kick live in `Hero::shake`, `kick`, `fov_add`, applied in
  `impact::apply_shake` after `sync_camera_transform` (it resets the field of view every frame). `hero keys space 1.9` holds Space
  1.9 s in a test (a full ground charge); the line of keys at the bottom is `controls::prompt`. The stacks lift F7's cap with the
  speed they give (`controls::stack_lift`; near a planet the cap braked a stack's speed back within 0.08 s). Past the barrier in air
  `Hero::buffet` (`controls::buffet`) drives a steady rumble in `apply_shake` and the speed lines (`readout::speed_lines`, egui's
  Background layer, drawn with the panels hidden too). The loops' files differ four times in loudness: each has its own
  `tune::LOOP_GAIN` and a ceiling `LOOP_MOST` that keeps its peak from clipping; change those, not `SOUND_GAIN`, to balance them.
- The lock-on (`viltrumite/lock.rs`, the user's 2026-11-07): in supersonic fast flight the left button selects (`lock::picks`;
  `hero_move` hands the punches an input without the left button then, so the two must agree: both go by last frame's flow); plain
  G is his while the mod is on (`camera.rs` `input_controls` skips its Go To; `ModInput::g_pressed`). A pick off the markers goes to
  the port's own sky pick as `CommandQueue::click` and is checked over the next frames (`Hero.pick_wait`). The trip estimate
  (`trip_time`) uses `step_speed` itself: change the speed rule there and the 5-minute reach follows. Messages from `hero_move` go
  through `Hero.note` (shown by `lock_keys`). The run and sprint clips are `run_clip` in convert.py (a solved stance foot, their own
  stride speed); the walk is still `gait_deltas`.
- Megaton Rainfall's assets (vegetation and cities plan): `se_core/src/mr.rs` reads its archives,
  models and materials, `se_core/src/dds.rs` `parse_dds` its textures (every mip and layer),
  `se_engine/src/mr_assets.rs` makes Bevy images and meshes (`MrAssets` resource). The port reads its
  own copy of the files it uses (`crates/se_engine/assets/mr`: the 16 trees, their textures, the
  generated species' barks and branch cards, the grass, the window array, the street furniture, 14 cars
  and their textures, `Data/global.txt`; 85 files, 57 MB; `MrInstall::open_copy`), the install (`--mr-path` / `MR_PATH`, default
  `C:\Users\liamw\Downloads\Megaton Rainfall`) only when the copy is missing. A new use of one of its
  files must add it to the copy: list it in `scatter::library`'s ignored test `copy_megaton_rainfall`
  and run it (`cargo test -p se_engine --release copy_megaton_rainfall -- --ignored --nocapture`); the
  `mr` tests still read the install. A model's own embedded material is the source for most props (global.txt lacks them);
  `MrInstall::resolve` picks. A few textures its models name are not in the game's files
  (`brick_1`..`6`): draw those with a neutral texture, never fail.
- GPU scatter (`scatter.rs`, `scatter/`, `scatter.wesl`, `scatter_draw.wesl`; the vegetation and cities
  plan's step 2): objects on the ground are placed and culled on the GPU from fixed cells (the quadtree's
  nodes split to at most 800 m) and drawn with indirect draws in the alpha-mask phase. Test with
  `SE_SCATTER_TEST=1` or the dev command `scatter test|off` (MR's tree_0 evenly on any planet);
  `SE_SCATTER_DEBUG=1` logs cells, pages and the buckets' instance counts. A tree stands on the drawn
  triangle: the cull pass rebuilds it from the node's vertices (`terrain::node_vertices`, as
  `build_node` makes them) and the geomorph (planet.wesl's vertex shader, `scatter.wesl`
  `drawn_vertex`). Change the ground's vertex positions, triangle split or geomorph only together with
  those, or trees float. The objects' light comes from `ground_light.wesl` (the light record, cloud
  shadows, terminator, haze), shared with planet.wesl: change the ground's lighting there so both keep
  it. The scatter's pipelines are built at start even when it is off, so a validation error in them
  quits the port.
- Civilizations (the plan's step 5): `Life { Type "Intelligent" Civilization 1..4 }`. Generated
  bodies get them in `se_core/src/procgen/civilization.rs`, run inside SE's life generator with its
  own hash: it must never draw from SE's streams (every generated system would change), which
  `cargo test -p se_core civilizations_only_add_tags -- --ignored` checks. Read a body's level with
  `BodyRecord::civilization()` (Sol's Earth is level 2 built in, its catalog unchanged) and count
  Intelligent as multicellular (`LifeParams::multicellular()`). `print_civ_stats` (ignored) re-fits
  the base chance and level weights to the decisions (1 in 20 land-life planets; 45/15/20/20). The
  dev command `search LY civ` logs every civilization the star browser finds.
- Cities (the plan's step 7): the layout is a WGSL function (`city.wesl`) in the cube face's frame
  (face, 4,096 m tile of face metres, the ground's soil coordinate in it: `PlanetUniform::city_tile`,
  the scatter jobs' `tile`/`local`); every period in it divides 4,096. The ground draws its roads
  (`city_ground`) and the scatter places one building a lot (`ct_building`): both use `ct_lot`, so
  change the lots only there. Buildings are a scatter class of their own (`Scatter::bcells`, jobs of
  kind 1 near within `BUILD_RANGE_M`, kind 2 far: the tall ones to `FAR_RANGE_M`, both drawing the
  same box where they overlap), their record in the candidate's noise slot, the box the library's last model
  (`ScatterLibrary::building`, the body's `species.w`), drawn by `vertex_building`/`fragment_building`.
  The towns' land is measured on `Terrain::ground_range_m` (SE's bump height), never the relief value
  (`proc_params.y` is not the height range). Earth's towns come from its lights map (the ground's tiles,
  the scatter's patch `veg_lights`). Near the ground a town's night light comes from its streets
  (`CityGround::lamp`); the lamps and the traffic are drawn by the ground too (`ct_lamps`,
  `ct_traffic`: no state, moved by the clock). The dev command `town [village|town|city|mega] [ALT_KM]`
  goes to the nearest settlement of the selected body's civilization (level 1 villages: `village_ground`,
  `ct_hut`, the huts and longhouses the box's next two models with the port's own textures in
  `assets/textures/village/`) (the settlement field on the
  CPU: `civilization::nearest_settlement`, same hash as the shader); follow it with `agl M` (the
  ground can stand kilometres above the radius). `town KIND ALT_KM PITCH` looks across a city.
  Level 3 (step 9): `scatter.wesl` `place_future` places the lanes, arcologies, megatowers, skybridges
  and the lots' boxes; the library's models after the box are city.wesl's `M_*` (3 the future box,
  4-7 the towers, 8 the arcology, 9 the skybridge, 10 the flying lanes, 11-13 the hero models; keep
  `BUILDING_MODELS` and library.rs's order in step). A building's record packs its size in 0.5 m steps
  (1 m for skybridges and lanes, 4 m for the arcology and landmark) and its height as the square root
  of a share of 1,600 m: change `put_building` and `cull_building` together. The hero models are
  `assets/models/hero/*.bin`, written by `make_heroes.py` beside them (run it in Blender: it models
  and exports them); their real sizes are constants in city.wesl (`TEMPLE_SIZE` and so on):
  re-export, then update those. Level 3's textures: `assets/textures/future/`. Every scatter point
  of a block or district must reach the same decision: decide a shared object (arcology, landmark,
  temple, megatower) from the town's density at its own middle (`town_at(city_dir(..))`), never the
  point's own, and clear lots only when it stands.
- Level 4 in space (the plan's step 10): colonies are chosen when a system loads (`system.rs`
  `colonies_of`, `ActiveSystem::home`/`colonies`; `se_core` `civilization::colonized`) and get level 3
  towns of the home's culture, under domes without air (`Terrain::domes`, `PlanetUniform::city_land.z`,
  the scatter's `flora_k.w`; `city.wesl` `ct_dome*`, decided by the unbent district cell). Stations and
  traffic are `orbital.rs` (`SE_ORBITAL=0` leaves them out, for comparisons; `s` the stations alone,
  `t` the traffic alone): the stations' models are `assets/models/hero/station_*.bin` (`make_heroes.py`,
  the layer in uv: 4 hull, 5 lights, 6 solar cells, 7 habitat), drawn with `HullMaterial`, whose lights
  are written only when the star's direction or the shadow changes. The traffic's per-frame values are
  records of one storage buffer written in place (`write_records`); never put them in its materials
  (changed every frame they cost a millisecond). The dev command `station N KM` looks at station N.
- A `perf NAME` segment runs until the next `perf`: put `perf jump` before a camera jump, or the jump
  and the next view count in the segment being measured. Test with `SE_CIV_TEST=2 SE_CITY_TEST=1` on a generated life world (towns on
  all suitable land), or Earth (New York 40.75 -73.98).
- Street furniture and 3D cars: the scatter's cells of kind 3 (`Scatter::bcells`, within
  `Levels::street_range`: 250 m at Ultra, 150 High, 100 Low; towns of level 2 and up, not under domes).
  `scatter.wesl` `place_street` reads the ground's layout the other way round: lamps where `ct_lamps` has
  them (streets and arterials), furniture between them, traffic lights at street corners. Cars are not
  stored: a record per grid square a lane passes through, and `cull_cars` works out every frame which
  cars' middles are in that square from the same hash and clock as the ground's `ct_traffic`
  (`ct_slot_car`, `ct_street_key`, `ct_art_key`; `street.y` is the ground's `globals.time`). So each car is
  drawn once and nothing moves in memory. Change the lanes in `city_ground` and `cull_cars` together; car
  speeds are in face metres (with true metres a car's place jumped where the metric changed). Models:
  `library.rs` `STREET_PROPS`, `STREET_CARS`, in step with `scatter.wesl` `S_*`; new files go through
  `copy_megaton_rainfall`. The furniture shares one palette texture (one material, whatever the model
  count), each car has its own texture; cars are turned nose to +x (Megaton Rainfall's face -x) with a dark
  patch under them; `street_surface` in `scatter_draw.wesl` draws them by `Model::flags` y. Models without
  impostors (buildings, street objects) take no impostor draw or atlas layers. Inside the range the
  ground stops drawing its flat cars and lamp heads, told through the light record's `glow_atmo.w`
  (`Scatter::street_range_on`), so they stay wherever the scatter draws none (colonies, `scatter off`,
  Cities Off, rural roads). A map body's scatter takes the ground's fixed grid spread 0.35 and sodium lamps
  (`Terrain::has_color_map`). The street draws are skipped while no cell of them is resident
  (`street_draw0`). A slot's "already placed" key holds the job's kind: a node's near buildings and
  street objects are the same cell. `SE_STREET=0` leaves them out for comparisons (the painted cars then
  show in the same places). Test: `surface 40.7498 -73.9796 0.3 -4 40; agl 4` in New York by day and night,
  and the hillside below.
- Roads on steep ground: the towns' streets and arterials are cut level into the ground's own heights
  when a node is built (`terrain.rs` `build_node`, `road_level.rs`, the layout's CPU mirror
  `se_core/src/procgen/city_roads.rs`). The layout in `city.wesl` (`ct_district`, `ct_grid`, `ct_local`,
  widths, seeds) and that mirror must change together; its tests compare them with numbers worked out
  from the shader. Levelled heights go to nodes of at most `LEVEL_SPACING_M` a vertex and into the normal
  maps down to `LEVEL_NORMALS_SPACING_M`; the geomorph's parent heights follow the parent's own rule, or
  nodes pop. The scatter stands on the drawn triangles, so trees and buildings follow by themselves.
  `SE_ROAD_LEVEL=0` leaves it out for comparisons. Test on a hilly white-star planet:
  `SE_CIV_TEST=2 SE_CITY_TEST=1`, `RS 8514-928-8-5906585-158 3`, `surface 26 -32 0.3 -22 40; agl 120` by
  day (Earth's data is too smooth to show it: Hong Kong's hills come out flat). Put `wait 200` before a
  first `rs` (the procedural stars load first) and `wait 800` before selecting its planet.
- Settlements seen from orbit (the plan's step 6): `settlement.wesl` (the field: Gaussian blobs in
  cells, each blurred to the pixel keeping its light; turned into the field's mean only once the pixel
  spans a good part of a cell, or towns vanish from orbit) and planet.wesl's civ block in `lit_ground`
  (generated planets only; Earth keeps its maps). The land's suitability says where towns stand (a soft
  threshold), never how dense they are. Their lights switch by the main star's light alone (`s.city`):
  in SE's light scale a moon's shine is ten times SE's city-light cutoff. Their brightness takes the
  self-emission setting (`levels.z`), never `levels.y` (0 without a glow map). The culture (lights'
  colour, towns' colour, coast) is `se_core::procgen::civilization::culture`, carried by
  `PlanetUniform::civ`, `civ_light`, `civ_urban`. The test planets orbit deep-red dwarfs, several
  tidally locked: look down on a night side with `surface LAT LON 15000 -90` (level 2's best is
  WISEPA J090649.36+473538.6 1 at 35 130).
- Vegetation and Cities settings (the plan's step 11): graphics.rs `Quality::vegetation` / `cities`
  (keys `Vegetation`, `Cities`: 0 Off, 1 Low, 2 High, 3 Ultra; `preset_scatter`; a Custom block saved
  without them gets 3), read by `scatter.rs` `Levels` (the trees' distances, grass range, buildings'
  ranges; a change starts the cells over) and by orbital.rs (Off hides the stations and traffic). Test
  runs use the High preset (2): put `levels 3` in a run to see or measure what the user plays with.
- Seasons on the trees (step 11): `scatter_draw.wesl` `tree_season`, broadleaf models only
  (`Model::flags.x`, from `ModelSrc::deciduous`: MR's trees and the species with branch cards), from the
  star's declination (`sun_body.y`), its trend (`sun_body.w`) and the axial tilt (`tint_gain.w`),
  worked out by scatter.rs `season_of` from the ephemeris (`wind.w` 1 when on; from the orbit that
  makes the seasons, a moon's planet's, never the body's own motion). Snow on trees comes
  from the ground's snow at their place (the cull's `plant_weights` w, SE's terra worlds only), packed
  with the two fades in `Inst::fades` (`pack4x8unorm`). The seasons fade out from half the trees'
  range, because the ground's forest colour has no seasons. The model record is `ModelGpu` (64
  bytes): scatter/gpu.rs's layouts take its size (`MODEL_SIZE`); change both shaders' `Model` with it.
- Vegetation (the plan's step 3, on by default, `SE_SCATTER=0` or `scatter off` turns it off): bodies
  whose catalog Life has multicellular land life (`scatter::land_plants`, `Terrain::land_plants`) get
  trees and grass. Where: on Earth (any body with a colour map) from a 512-texel patch of its map round
  the camera classified into forest, grass and steppe (`scatter/earth.rs` `classify` and
  `vegetation.wesl` `veg_classes`: keep them the same) and sharpened by the woods' noise; on SE's
  terra worlds by the ground's own biome weights, which live in `surface_class.wesl` (shared by
  planet.wesl and the cull pass: change a biome rule there and both follow). The ground darkens the
  forest floor under the trees and draws the crowns past their 3 km range (planet.wesl `lit_ground`,
  `vegetation.wesl` `canopy_shade`); it learns the body's vegetation from its light record
  (`PatchLight::flags` z the kind, w the trees' range, written by terrain.rs). The trees take the
  ground's colour where they stand over their own mean colour times a gain matched by pixel means to
  the ground with the trees off (`BodyFrame::tint_gain`; re-match it if the ground's colours change).
  Grass is the same scatter in 40 m cells within 120 m (4 tufts built in code with MR's grass atlas).
  Species (step 4): the library's models are MR's 16 trees, Earth's 8 generated species, the active
  planet's flora (8, rebuilt with their impostors on arrival: `library.rs` `update_flora`) and the
  grass, in that order; the cull picks a tree's species from its climate (scatter.wesl
  `earth_species`, `flora_species`, by the places listed at `trees::SPECIES`: keep them in step). The
  port's own leaf cards are in `assets/textures/vegetation/` (power-of-two squares, alpha cut-out).
  The meshes' light is worked out once per instance by the compute pass `light` in scatter_draw.wesl
  (per vertex it cost 0.9 ms on a forest floor).
- The mod's effects in the air (`viltrumite/fx.rs`, `hero_fx.wesl`; spec F9): one additive pass after the clouds (`CloudsPassSet`), no depth
  attachment, no vertex buffers, one record buffer written in place; it reads the depth for a soft edge and the clouds' `CloudTextures::drawn`
  for what's behind them. The look is in `tune.rs`; `hero cruise V [M]` holds a speed and height whatever the view does (`hero aim` then goes round
  him), `hero boom FILE` logs the air and the barrier, `hero air` prints a body's air and F9's table for it. Scenes: `.optimize/bench/hero_fx.sh`,
  `hero_stages.sh`, `hero_dive.sh`, `hero_heat_table.sh`, `hero_fx_cost.sh`. A `shot` costs about 30 frames of a run, so plan pictures of a fast event
  with no `wait` between them. `perf.rs` drops passes under 0.05 ms: `hero_fx` is under that. `giant_dark` (volumetric_clouds.wesl) must keep leaving
  out what's within a few hundred metres of the camera, or the view's dark deep in a giant hides him 6 m away; his own light there goes by
  `draw.rs` `deck_light`, measured against the decks' brightness: re-measure if those change. The trail's points die out of order (one laid
  as he cools lives less): `FxState::alive_until` says whether any lives, and `trail_records` keeps a dead point in the ribbon at no light;
  never stop its walk at the first dead point (the whole trail vanished at once). Points are laid at least a 254th of the longest life apart,
  so the ring holds a life at any frame rate. The cone, its pulse and the ring go by the barrier's air (`fx::air_show` takes the pressure:
  nothing under 100 Pa; without it a lit cone hung round him in near vacuum). A plain Go To's path holds still in the universe's axes, so
  `hero.rs` `carried_velocity` takes the body's turn past x1 out of a carried speed (at x1000 it read 465 km/s over Earth's equator); Land
  on turns with the body. A run's command queue waits while a Go To lasts, so no picture catches one in mid-flight: use `hero boom`'s log.
  Under a giant's radius his air goes on denser and hotter (`air.rs` `AirModel::deep`, SE's polytrope; `AtmoParams::state`, the ship's,
  holds the radius's 1 atm all the way down): Jupiter 17 atm at -100 km, where he falls at 63 m/s.
- The mod's impacts (`viltrumite/impact.rs`; spec F11, step 8): `impact::start` reads the buttons before the frame's keys and before
  `Frame::carry` (a ground punch from a jump or a fall turns him to Hover, and a state changed after the carry throws him by the ground's
  turn, 7.75 m at Earth's equator), and `impact::act` runs the action after his step (`Hero.action`, its layer the sampler's slot 4, its clock its own: rate 0); a ground punch or a clap holds
  his keys (`Action::locks`). What an impact leaves goes into `Hero.blasts` (8, drained every frame by `update_impacts`), never straight
  into the pools. The pools (`Impacts`: 32 craters, 64 bursts, made at first enable) run while he's on or while they hold anything
  (`ModControl::aftermath` is "the pools hold something", set by `update_impacts` even while he's on, or turning him off would stop them
  before they're marked); a crater is freed only when both he and the camera are `CRATER_FREE_M` (the buildings' `FAR_RANGE_M` + 500)
  away, the camera alone in the aftermath, and never faded before (D3). Anything placed from `hero_move` must use the frame's position
  this frame (`Frame::world(scene, ..)`), never `Hero.world` (last frame's: Earth moves 500 m a frame, and the clap's dust landed 480 m
  off). Craters: `fx_decals` (`hero_fx.wesl` `decal_*`) multiplies the scene before the clouds, from the depth, so it follows the drawn
  ground and takes its light; a crater's middle is re-read from the ground each frame. It leaves alone what stands up (derivative normals:
  computed before any `return`, or the WGSL won't validate) and him: capsules round his bones as posed this frame
  (`impact::BODY_PARTS`, `BodyParts::place` in `update_hero`, `fx::PARTS` at most; an upright cylinder missed the ground punch's fist),
  and the sea (what stands under `CRATER_SEA_M`, 1 cm, over it: Earth's flat coasts are a few cm over the sea). A crater landing in a
  live one grows it (`CRATER_MERGE`): each multiplies the scene. Dust: `fx_pass`'s first draw, premultiplied over; its colour is read from the lit scene round each burst by `fx_dust_tint`
  (a compute pass before the craters, its buffer keyed by the burst's seed), so it's the ground's own colour, light, night and cloud shadow.
  Rims are `RimMaterial` (`hero_rim.wesl`), lit by his light record, never changed (not `HullMaterial`: it has no cloud shadow). In the
  aftermath that record (and `fx.sun`) is worked out at the crater or dust nearest the camera (`draw::aftermath_light`, chained between
  `update_impacts` and `fx::upload`, the light pass on while there's one): frozen at the hand-back, rocks stayed sunlit into the night.
  The rims carry no `UPosition`: `update_impacts` sets their `Transform` from the camera itself (the floating origin would rewrite all 32
  every frame, the mod off too). The rims' pipeline is built by one rim kept as a speck at the camera for `RIM_WARM_FRAMES` while he's on
  (`hand_back` hides it). The shake turns the clouds' view too (`volumetric_clouds::ViewShake`, read only in a frame it's written). Dev verbs: `hero crater [R]`,
  `hero dust [S]`, `hero splash [S]`, `hero impacts` (logs the pools, works in the aftermath). Scenes: `.optimize/bench/hero_crater.sh`,
  `hero_actions.sh`, `hero_splash.sh`, `hero_keep.sh` (kept, freed at 20.5 km, the mod off), `hero_impact_cost.sh`.
- The mod's buildings' probe (`scatter/probe.rs`, `viltrumite/breakable.rs`; spec F12, step 9): the scatter's cull lists the buildings in a
  capsule round him (`scatter.wesl` `probe_building`, from `cull_building` after the range fade, before the view's planes) into binding 20,
  copied to one of six staging buffers made at start and read back 3-5 frames later (`BuildingProbe::list`, body frame; `age_frames()` is its
  age now; the list goes when the scatter stops taking the asks: at once when it's off or has no body, after `UNSERVED_FRAMES` otherwise;
  a frame with no idle staging buffer is counted, `skipped`). `ask_probe` and `receive_probe` run only while the mod asks (`run_if`: off
  costs nothing). Bindings 19 (the
  broken table, step 10) and 20 exist with the mod off and are in the compute layout every scatter pipeline is built with at start: a
  runtime-sized array's binding must be at least its header plus one element (64, 96), or validation quits the port for everyone.
  `body.street.z` is the probe's flag (0 with the mod off: one uniform branch; 1 as drawn; 2 measuring, both lists in full): never reuse it.
  A building's id is its face (`Cell::face`, the spare float) and its middle's face metres x 4 from the face's corner (`cell.tile` whole,
  so no f32 rounding). Near and far cells agreed within 0.25 face m (measured), but a box's middle is not exact: its front and back setbacks
  follow `down` (`city.wesl` `ct_building`: `core`/`urban` read at each cell's own grid point), which moves it along its lot's depth by up
  to 2.5 x the change of `down` / k face m, about 2.3 face m at a cube corner. So match within +-4 face m (`MATCH_UNITS`) and the same
  model (or by footprint); the record's hash differs between them. Heights and existence differ more. Change
  the building cull's positions, the cells' `tile`/`local`/`edge` or the records' layout (`ProbeRecGpu`, 48 bytes) with the probe. Each
  list is turned back by its frame's camera, body and turn (an 8-entry ring by `seq`): never by this frame's. `timing frame` steps
  `Time<Real>` too, so measure real milliseconds without it. Verbs: `hero probe log|list|trace|stats|anchor|both`; scenes
  `.optimize/bench/probe_flight.sh` (near-far-near, the capsule anchored), `probe_counts.sh`.
- Keep test runs short (the user, 2026-11-04): the ground loads in about 30 s after a jump, so wait
  about that (`wait 1500`-`2400` with `timing frame`, which runs as fast as it can), take the picture
  and end the run; never a fixed wait of several minutes "to be safe". The star browser's search
  (`search LY life|civ`) finishes in about 30 s (it stops at 10,000 systems): end the run then.
- Test runs' `shot PATH` needs a Windows path (`C:/...`): a Git Bash `/c/...` path inside `--run` is
  not converted and the picture is not saved. After a camera jump let the ground load before judging
  a picture (`wait 1200` or more): its fine nodes arrive over hundreds of frames.
- Shaders are loaded at run time from `crates/se_engine/assets/shaders` (no rebuild needed to
  try a shader change), but an edited shader takes a few hundred frames to compile in a test
  run: the ground is missing until then, so wait ~900 frames before judging a screenshot.
- Judging screenshots (the user's rule, after Claude fooled itself twice on 2026-11-03):
  - Make sure a screenshot is from THIS run: the port exits a few frames after requesting it
    once the file exists, so an old file with the same name used to stay (runport.sh now deletes
    it first; still check the run's log says "Screenshot saved" and the file's time).
  - Let the game load fully first. After any shader edit (planet.wesl above all) the edited
    pipelines compile for a long time and the ground or clouds are missing meanwhile (a milky or
    hazy picture with no ground is that, not a lighting bug): wait 2400+ frames, and compare with
    a run of the unedited shader before blaming the change.
- Disk space: what grows is `target/` (`target/agent`, the `--profile agent` builds, reached 6 GB;
  `target/debug`; old unused files in `target/release/deps`), `target-trace/` and the test pictures in
  `.optimize/scratch/run` (up to 8 MB each). All of that can go, builds come back by themselves. Keep
  `target/release/spaceengine-rs.exe` (what the user plays) and the folders git does not hold, which
  nothing can restore: `re/` (the Ghidra project, SE's shaders, the notes on other games) and `.audit/work`.
  Use the Recycle Bin, never a hard delete: emptying it is the user's step. A full release build is slow, so
  to thin `target/release/deps` list what is in use (`cargo build --release --message-format=json` and
  `cargo test -p se_engine --release --no-run --message-format=json`, the `filenames` of each artifact) and
  take only the rest; `cargo build --release` should then compile nothing.
- Test runs: `bash .audit/tools/runport.sh OUT.png FRAMES "cmds"` (one run at a time). Useful
  dev commands: `select NAME`, `surface LAT LON ALT_KM PITCH`, `agl M` (hold M metres above
  the ground), `fly KMS`, `spin DEG` (turn in place), `orbit DEG` (swing round the selection, as the
  right mouse button does; degrees a second), `photo 0|1|2` (photo mode: 2 is auto exposure), `exposure STOPS` (compensation, to measure dark pictures: a night's trees are level 0 or 1 in a PNG), `ambient X` (SE's
  ambient light), `shot FILE`, `wait N`, `date`, `pause`, `mods list|on NAME|off|window [off]|sound NAME` (the Mods menu; `sound` plays `sfx/NAME.ogg`, the log says decoded, playing, finished), `hero keys KEYS SECS|aim Y P|state|trace FILE [BODY]|orbit K|clip NAME|light` (the mod's hero: `hero keys w+shift 3`, `hero keys off 0`), `script PATH`
  (start a script, a file on disk too), `uiclick X Y` (a real left click for egui at that window point: the menus' buttons
  can be tested). A 1x1 screenshot
  (72 bytes) means the window was minimised at that moment: take each picture twice.
- The user's own `se.log` and `settings.cfg` (in `%APPDATA%\SpaceEngine-rs-v2`) can be hidden
  from Claude's shell by the desktop app's sandbox (they were readable on 2026-11-02); if so, copy
  them out with a process started through `Invoke-CimMethod Win32_Process Create`.
- Testing settings save/load (scripted runs skip the settings): run the exe unscripted with
  `APPDATA` set to a scratch folder and `timeout 60` (first run writes `settings.cfg` there at once).
  Edit that file and delete its `Version` line, run again: the port rewrites the file from what it
  loaded. Never point such runs at the user's real folder.
- Volumetric clouds (`volumetric_clouds.rs`, `volumetric_clouds.wesl`, plan in
  `docs/spec-volumetric-clouds-weather.md`): full-screen passes in `Core3d` after the main pass,
  before render_scale's `SolidUpscaleSet`, drawing into the solids' corner of the main texture. They
  read the depth buffer (the main camera's `depth_texture_usages` includes TEXTURE_BINDING). The
  active body's flat cloud shells are hidden only while the passes draw it (`VolumetricClouds::body`,
  read by terrain.rs). Positions go through "sphere space" (body frame / ellipsoid semi-axes) and the
  camera's altitude comes from f64, so ray-sphere maths stays exact near the surface. GPU time shows
  in `SE_PERF` as `volumetric_clouds/elapsed_gpu`. The cloud cover generators (SE's terra clouds, the
  port's procedural ones) live in `cloud_cover.wesl`, shared by the flat shells (`clouds.wesl`) and
  the volumetric cover map: change them there and check the flat clouds stay identical. The passes
  read the depth buffer as true distances, so their body must be in linear space, not scaled space.
  `VolumetricClouds::active` makes camera.rs `update_nearest` stretch the linear radius over it: a
  nearer moon once left the planet compressed and its clouds missing (fixed 2026-11-03). Ground shadows:
  `planet.wesl` `cloud_shadow` reads the cover map, the shadow record and the shadow map
  (`CloudShadowAssets`, bindings 27-31 of `PlanetMaterial`) and the weather cube (32, with the cover's
  sampler). The shadow map (`shadow_gen`, 40 km around the spot under the camera) and the cover's
  shadow farther out must give the same light, or a bright square of ground follows the camera (it did
  three times): both a quarter under thick cloud; near the camera the ground takes the map's shapes at
  the cover shadow's mean darkness (the map over its last mip, 8 texels of 10 km, times the cover's
  shadow: a patchy cloud lets more light through than an even one); both follow the star's ray over
  the round body (`cloud_cover.wesl` `climb_dist`: over flat ground a low sun's ray passed above the
  clouds); both go on past the terminator (cut there, the shadows ended in a line).
- Weather (`se_core/src/weather.rs`, `volumetric_clouds/weather_sim.rs`, `weather_sim.wesl`): the
  derived weather is a pure function of climate, place and date; its maps (two dates 10 simulated
  minutes apart, weather + wind layers) are made on a worker thread, at once on arrival or a date
  jump. A GPU simulation (cube 6 x 128 x 128: storm activity, stable air, anvils) runs inside
  `clouds_pass` before the trace and writes the weather cube the clouds and the ground read. Its rules
  (`SimClock`): live within 20 radii up to x1000; faster, farther or backwards it follows its seed state;
  paused it is frozen; a new body or a date jump seeds it at once. Near the camera the noise drifts with
  the wind (`CloudView` `noise_off`/`detail_off`/`motion`, also used by the reprojection).
  `SE_WEATHER_SIM=0` leaves the simulation's cells out (comparisons); `SE_WEATHER_NEUTRAL=1` turns
  the derived weather off as well (the cover exactly as SE's map or generator makes it). The cover a
  layer gets is `cloud_cover.wesl` `weather_cover` (the weather and the opacity on SE's cut cover:
  `se_cut` once, on raw map or generator values; the cover map is stored already cut), used by
  the march, the rain and the ground's cloud shadows: keep them on it. The weather must keep SE's
  cloud map recognisable: stronger terms once made Earth's storm track a beige overcast from orbit.
  `cargo test -p se_core print_cover_stats -- --ignored --nocapture` prints the weather's effect. Dev commands for weather tests:
  `timing frame` (a fixed 1/60 s a frame, so runs reach the same date whatever their speed), `resume`
  (undo `pause`), `switch KEY on|off` (a render switch such as `ShowRain`, for on/off comparisons).
- The clouds' plan since 2026-11-03 is `docs/spec-clouds-v3.md` (one cloud system, the Flight
  Simulator look, speed first). On High and Ultra the ray-march draws the active body's clouds at
  every height; SE's flat shells only cross-fade with it between 40 and 60 radii (`CloudView::lod`,
  `VolumetricClouds::flat_hidden`; there clouds.wesl reads the weather cube and the `CloudShadow`
  record: key.w the weather's share, sun.w the shells' share, layer_b.w the layer index + 1), and it
  fades in over 1 s once ready. The earlier hand-over (flat shells from orbit down to 15x the cloud
  tops, docs/spec-cloud-lod-and-popping.md) was dropped: while the flat shells drew the clouds, the
  trace's rain shafts showed through them as dark patches, and it saved nothing near the ground.
  Whatever lies under the clouds (rain, fog, dust) must stay in the same march as the clouds, so they
  hide it. `SE_CLOUD_LOD=0` is the old hard switch without fades (comparisons).
  The cover map is 2048 a face (`COVER_SIZE`; catalog maps composed from SE's level-4 tiles, a thread
  a face). Each layer gets a stack of detail cover patches (`volumetric_clouds/detail.rs`,
  `DETAIL_LEVELS` 3, 2048 texels, Earth 800 / 1,600 / 3,200 km across, in the layer's frame, each made
  below 3.5x its half-width and re-centred after a quarter of it; catalog maps from their deepest
  tiles on a worker thread, generated layers by `cover_patch` on the GPU, one a frame); `cover_at`
  takes the finest that holds a point at its footprint (`CloudView::detail`, binding 19). The near
  pass traces the first 4 km of the rays (Ultra; High 2 km), at half the solids' size, a quarter of its texels a frame, with its own
  history, over the far trace (`CloudSize::params.w` the ray part, `CloudView::lod` z/w). The far
  trace does one texel of each 2x2 block a frame in turn (`CloudSize::trace`, `SE_CLOUD_QUARTER=0`
  all; the resolve reprojects each texel by the trace's distance, which must be the real scene
  distance for a clear texel: capped at 10,000 km, orbiting a planet turned its clouds into blocky
  ghosts, and a texel whose ray misses the cloud shell is always clear); the march takes long steps through clear air (`maybe_cloud`: never step back over what was
  sampled, that once hid whole decks) and the light from 20-40 km on comes from the cloud above
  (`light_column`, the area's mean: a low sun crosses the gaps). The composite sharpens far clouds to
  full resolution by the cover (`cover_tau`, the resolve's second target). The weather's own cloud is
  shaped by a baked pattern map (`weather_pattern`, `CloudShadowAssets::pattern`, binding 27; planet
  material binding 33) through `cloud_cover.wesl` `weather_shaped`: the march, the rain and the
  ground's shadows all use it (the flat shells not yet). `SE_DETAIL_COVER=0`, `SE_NEAR_PASS=0` leave
  parts out; `SE_CLOUD_REFERENCE=1` draws the reference to judge shapes and light against (every pixel
  at full size traced every frame over the whole ray, twice the steps, no history: slow); the dev command `vclouds 0|1|2` switches the 3D clouds off/High/Ultra within a run (flat
  vs 3D pictures; note the flat shells then draw without the weather). The march (`trace`) is nearly
  all of the clouds' cost; the worst case
  so far is the user's planet RS 8514-3620-1-7-69 1 (one layer, then a high veil at 14-18 km, and the weather's cloud): 11-16 ms
  at Ultra 3440x1440 before the plan, 1.4-2.7 ms after its step 4 (`.optimize/bench/rs8514_ultra.sh NAME "extra commands"`; planet 2's
  thin high layers ~5.5 ms looking around). Since 2026-11-04 that layer is cumulus heaps at 10.7-17 km, 1.2-1.8 ms.
- Layer heights (`layer_volume`) count from the ground's middle (`Terrain::ground_mid_m`: the middle of
  the relief, never below the sea; SE's generated layers stand on it). Every body's lowest layer is the
  lower atmosphere's cumulus, as Earth's (at most 9 km deep; an opaque map like Venus's stays a sheet).
  Once only a lowest layer under 15 km above the radius was cumulus: dry worlds with 20 km of relief
  (the ground's middle 10 km up) then got veils alone (the user: "procedural planets are missing the
  detail"). Earth's middle is 0.
- The far trace's steps grow with the distance from the camera (`step_rate`: q t, q bisected so the
  steps cover the ray's pieces inside the cloud bands, at least 0.004 t), and a sample's footprint is at
  least a quarter of its own step. The march goes through the cloud bands alone (`ray_piece`,
  `CloudView::bands` from `cloud_bands`: the layers' heights merged where they overlap), skipping the
  gaps between layers, each piece from its own jittered start. With the whole shell from the lowest
  layer to the highest as its path, a moon's heaps under veils 70 km higher got km-long steps: rings
  round the spot under the camera, heaps that appeared only lower down. Cloud density must stay inside
  each layer's bottom..top (the march never samples outside the bands). Each piece gets at least 8
  steps and starts jittered over its whole first step (`march_dt`): with 2 km steps over a 4 km layer
  and a start jittered over less, whole features came and went as the camera's height changed. With even steps (path / MAX_STEPS) a ray skimming a layer from inside it (150-300 km) had km-long
  steps next to the camera: the heaps there became the layer's mean, a flat band over the far heaps'
  tops that zooming in magnified (the user's 2026-11-04 video, a 10 degree view). The near pass keeps
  even steps.
- Night light: the clouds take the further lights as the ground does (`BodyLighting::extra`: other
  stars and planet shine; `CloudView::extra_dir` / `extra_light`, `night_light` in the shader, no light
  march of their own; their haze too) and SE's `AmbientLighting` (`sun_light.w`; the flat shells take
  it as well). The sun's light march is skipped where the sun has set (`tw_l` 0). Without these the
  night side's clouds were black over a moonlit ground. Test with `photo 2` (auto exposure, as the user
  plays) and Earth under a full moon (2026-11-24 14:53, 20 N 137 E is midnight under it).
- Lightning (`weather_sim.wesl` `sim_flash`): 6 of the 8 slots look for storms only around the camera
  (300 km or its height), 2 anywhere, each the most active storm of its 32 tries by the storms' own
  activity (the fx cube's B, written by `sim_out`), not the weather's convection (low at night even in
  a storm). The derived weather has convection at night (se_core weather.rs: half the afternoon's at
  midnight). Before, no flash ever went near a camera on the night side.
- Cloud shapes (the clouds plan's step 5): cumuliform layers are fields of heaps (`layer_density`,
  `heaps`): the shape noise, spread evenly over 0..1 by a logistic fitted to its distribution (about
  0.833 +- 0.041: se_core `cargo test -p se_core print_shape_quantiles -- --ignored --nocapture`; change
  the shape volume and refit it), read on the layer's base in two layouts (one turned: the 6 km tile
  alone lined the gaps up in rows), picks where heaps stand and each point's strength; the types blend
  their tops (`type_top`, `type_height`, a heap's height from its width) by weight, never their
  densities (that stood faint 8 km towers over every heap). Keep `heaps_ceiling` (the clear-air test's
  limit) and the mean curves (`type_density`, the far mean and the light from above) in step with
  `heaps`. Cloud types come from the regional cover (`regional_cover`, the cover map's mip 2), not the
  local one: SE's maps give each cumulus blob a cover near 1. The weather's own cloud (`weather_shaped`)
  must fade out with its share, never switch on at a level: a switch at 2 % cut its clouds into
  saw-tooth walls along the weather cube's texels.
- Cloud light (step 6): the light volume (`LightVolume` in volumetric_clouds.rs, `light_gen` and
  `light_at` in the shader, bindings 28-30 of the trace's group): 160 x 160 x 32 over the layers within
  60 km of the spot under the camera, two of them, the back one built 4 slices a frame, swapped when
  complete; only while the camera is within 30 km of the clouds' top. The march takes its first three
  light steps itself, the rest from the volume. The march (`trace`) is compiled twice (`HEAPS` set or
  not, `CloudPipelines::trace`): code compiled into it costs registers even where it never runs (the
  solid types' code slowed a body of veils alone by a third), so measure the user's planet
  (`.optimize/bench/rs8514_ultra.sh`, heaps) and a body of veils alone after adding to the march.
- Weather effects (step 7): the trace draws rain/snow shafts, fog and dust (`media`) and lightning's
  glow in clouds; `weather_fx` (full resolution, after the composite) draws near rain streaks or snow
  flakes and lightning's light on the ground. A second cube holds fog and dust; `sim_flash` picks 8
  flash slots into a storage buffer. The effects' clock (`Drift::fx`) runs at most at real speed and
  stops when paused. Bodies with an atmosphere but no cloud layers also get the passes (fog, dust).
  `SE_WEATHER_DEBUG=1` draws a weather map at the top of the screen (red rain, green convection, blue
  fog, orange dust, cyan flashes, white dot the camera): use it to find test spots before taking
  pictures. Flashes last about 0.3 s, so a single screenshot rarely catches one.
- The Viltrumite mod's sounds (`viltrumite/sound.rs`, spec F10): only `play_sounds` spawns audio. Whatever happens calls one function on
  `Hero.sound` (`Cues`), once, the frame it happens (`boom`, `punch`, `thunderclap`, `building_breaks` and so on are ready). Levels read
  `Hero.flow` and `Hero.heat_k` from `air.rs`, nothing else. Stop a voice by despawning it (removing `AudioSink` restarts it). **`wind_loop`,
  `superWind_start`, `wind_end`, `wind_attack_*` are the source mod's super breath: never use them for flight (the user, 2026-11-07).**
  `convert.py` refuses them and `no_breath_sounds` fails if one comes back. A new sound: `sounds!`, `convert.py`'s lists, then
  `python -I re/viltrumite/tools/convert.py --only-sounds NAME`, which also updates `mod.cfg` (every sound must be a `Need`: that's what
  checks it on first enable, and bevy_audio panics on a bad file). No test can listen: `hero sound` logs each loop's volume, pitch, paused
  state and position, `hero sound trace FILE` logs every frame (`.optimize/bench/hero_sound.sh`, `hero_shots.sh`), `hero sound cue WHAT`
  fires a trigger's function. `timing frame FPS` steps by 1/FPS, for frame-rate tests. `print_sound_levels` (ignored test) gives each file's
  real loudness: they differ by ten times.
  Where there's no air everything of his is silent (D12, the user's 2026-11-12: every loop and one-shot times `air_share`, 0 under 1 Pa;
  a silent one-shot is counted but gets no voice, and a playing one follows the air every frame, `Shot::air`); the sandbox's
  sounds and the music don't go by it. `Director::tier` follows the speed
  in every state, a fall's too: what's about Hyper flight goes by `hero::flying_hyper`. A Go To carries him with no velocity of his own: `Hero.flow` and
  `Readout.speed` take the camera's speed over the ground (`Hero.carried_from`), `speed` in `hero_move` stays 0 for his poses. `.optimize/bench/hero_sound_cases.sh` has these cases.
- SE's gas giant cloud generator lives in `gas_giant.wesl` (noise primitives, `gg_height`,
  `gas_giant_cloud`), shared by `planet.wesl`, `brown_dwarf.wesl` and the volumetric giant decks.
  Change it there and check flat Jupiter stays identical (it was pixel-checked when split out).
- Gas giants' volumetric decks (`giant_specs`, `giant_sample`; on by default, `SE_GIANT_DECKS=0` off)
  fade in within 6 scale heights of the cloud tops and must match SE's flat bands there: a deck point's
  light counts the cloud above it from the main deck's own top there (`giant_sample` puts that column in
  the sample's `wx.x`), never the deck's mean top (that made belts five times too dark); they take the
  atmosphere's haze down to the radius (the tables' ground) like the ground; a catalog giant's colours
  come from its own colour tiles (`compose_face` with a tint, cover kind 5 with `kind.y` 1), else SE's
  generator. The rings shade every body's clouds (`ring_shadow`, in the body frame: in sphere space a
  flattened giant's shadow fell in the wrong places) and SE's rings winter fades a giant's deck. The
  camera may go down to 10 scale heights under a giant's radius while the passes work on it
  (`giant_floor_m`, terrain.rs `keep_camera_above_ground`; `trace` takes out of the depth only the flat surface, by the ray's own hit with the radius, so a ship or the hero in front of it stays in front of the decks); below the radius terrain.rs hides the
  giant's flat bands (seen from below, their nodes' skirts were a grid of lines). A hot giant's decks
  glow as its bands do (`deck_glow`, `CloudView::glow` from terrain.rs `thermal_uniform`; hotter with
  depth below the main deck's top, `giant_sample` puts that depth in `wx.y`), and a glowing atmosphere's
  light is in the decks' haze (`glow_atmo`). Giant lightning: `sim_flash` with
  `flash.w` 2 (`giant_storm` cells, flashes about the main deck's base). Compare with `SE_GIANT_DECKS=0`
  and `photo 0` (fixed exposure); find a giant's day side first (look down from 3,000 km at four
  longitudes).
- The clouds' weather makes the user's planet 2 spot (25.46 N, -48.19) clear: it lies in a dry
  subtropical belt. Cloudy test spots change with the date; check orbit pictures first to find one.
- Reaching the user's RSN planets by dev commands (`.optimize/bench/rsn_ultra.sh`) needs the
  `--size 3440x1440` window: at the default size the nebula's star was "not found".
- Python on Windows: `read_text`/`write_text` turn LF files into CRLF. Patch files with
  `read_bytes`/`write_bytes` (and keep each file's own line endings: `git ls-files --eol FILE` says which it has; the repo is
  a mix, and `grep -c $''` counted every line of LF files here).
- wgpu's `Operations::default()` CLEARS the attachment: a pass that blends over the scene must use
  `LoadOp::Load`.
- Reverse engineering: Ghidra project in `re/ghidra_projects/se`; decompile with
  `python -I .audit/tools/dec.py FUN_x`; SE's decrypted shaders are in `re/se_shaders`.
- Flattened planets (Earth, Saturn, fast-spinning giants) are drawn on an ellipsoid. Any shader
  maths that uses a sphere (atmosphere tables, camera height) must undo the ellipsoid per axis
  first, as `planet.wesl` `table_local` and the sky shell do. Dividing by one radius left
  flattened giants mostly flat blue.
- Reaching a star in another galaxy with dev commands: `select RG …` twice, `go <km inside it>`,
  `wait 600`, then `rs RS …`. The planet names then work with `select`. A nebula's star (`RSN …`):
  from inside the galaxy (`go 30000000000000000`), `goto RN …` twice with `wait 1500`, then
  `goto RSN …` (put a harmless command such as `spin 0` before it after a long wait: the name
  resolver gives up after 3000 frames of a name waiting at the front of the queue).
- Test runs skip the user's settings: they use the High preset (solids at 85% of 1080 lines, no
  MSAA) and a 1600x900 window. The user plays maximised on a 3440x1440 screen (RTX 5080), since
  2026-11-04 with Custom settings (no MSAA, solids 85% of 1080 lines, VSync on, Ultra clouds; it was
  Ultra with 4x MSAA at full resolution, several times the GPU time). Measure what they see with
  `--size 3440x1440` and `vclouds 2` (or the `preset Ultra` dev command for the old settings,
  `.optimize/bench/rsn_ultra.sh`).
- Ground shader: on procedural worlds' fine nodes the colour generator's kilometre-scale noises are
  worked out per vertex (`planet.wesl` `SELENA_AT_VERTICES`, `TERRA_AT_VERTICES`,
  `TERRA_CLIMATE_AT_VERTICES`, chosen per node by terrain.rs `vertex_noise`), coarser nodes per
  pixel; both paths must keep calling the same `selena_noise` / `terra_noise` /
  `terra_climate_noise`. A new noise term with features finer than kilometres stays per pixel (or
  gets its own frequency in `vertex_noise`). Terra nodes too coarse for all of `terra_noise` split it
  (`TERRA_PARTIAL`, since 2026-11-04): the vertices carry its warp and each sum's coarse octaves
  (`vertex_octaves`, passed to the pixels as a flat value so the two stages never disagree), the
  pixels add the finer ones (`ter_fbm_from`, the same points and amplitudes as `ter_fbm`), and a pixel
  that keeps fewer octaves than the vertices works the whole sum out itself. `from` is a reserved
  word in WGSL (a shader using it fails to compile and the ground vanishes).
- Ground node skirts (the walls hung under each node's edges that close the cracks to coarser
  neighbours) are a mesh of their own, a child entity of the node, with the node's material cloned
  with `skirt: true`: `PlanetMaterial::alpha_mode` puts it in Bevy's alpha-mask phase, drawn after all
  the opaque ground in the same pass, so hidden skirt pixels fail the depth test before they are
  shaded. Bevy orders opaque draws by pipeline and material, not by distance: drawn with their node,
  the skirts were shaded wherever they came before the neighbour hiding them, a third of the ground's
  GPU time near the surface (2026-11-04). Keep them apart, and never `discard` in the ground's
  `fragment` (it would turn the early depth test off).
- What the ground costs (2026-11-04, Ultra 3440x1440, `main_opaque_pass_3d`): found by switching one
  part of planet.wesl off at a time in a copy of the exe with its own `assets/shaders` beside it (the
  exe reads those first; run the benches with `PORT_BIN=` that copy, so the repo's shaders stay as
  they are). The pixel shader alone is nearly all of it (a flat colour: 2.5-3 ms is raster, vertices
  and MSAA). Near the ground on the user's planet: the soils (within 2 km) ~7 ms, the sample layers
  3-4, the haze 1.7, cloud shadows 1.6, the normal map's jitter 0.3, sky light 0. From altitude SE's
  colour noise per pixel on coarse nodes is about half (the moon 5.1: the colour generator is ~80% of
  the ground from 13 to 350 km). Code that never runs cost nothing measurable (a far-only shader
  without the soils, a terra-only one: no gain). Probes that change the colour also change which
  soils the ground picks, which moves the soils' cost: read them with care.
- Measuring speed: `SE_PERF=FILE` turns on the frame-time logger (`perf.rs`); the dev command
  `perf NAME` … `perf end` marks a segment (it runs until the next `perf`: rs8514_ultra.sh's
  `km114` also holds the jump to 3 km). The scripts in `.optimize/bench/` (`all.sh` scenes,
  `refs.sh` reference shots, `compare.py`) do this for fixed scenes; `.optimize/REPORT.md` has the
  2026-11-02 optimization results. A segment whose median sits at ~16.6 ms was capped at 60 fps from
  outside the port (window not in focus): rerun it with the window in front. A segment with passes'
  GPU times missing or tiny (seen once with the clouds off) is broken the same way: rerun it. The
  clouds' parts show as `volumetric_clouds/sim`, `shadow`, `trace`, `resolve`, `near`, `fx`.
- GPU pipelines (`pipeline_watch.rs`): Bevy builds each one the first time something draws with it,
  and nothing using it is drawn until then; the NVIDIA driver takes 5-6 s for each planet.wesl
  variant. The ground's variants (terrain.rs `VertexNoise` x ground / skirt / sea) are built ahead at
  start (`WARM_ORDER`: add any new variant there), and a node draws with `VertexNoise::None` until
  its own is built (`NodeNoise`, `upgrade_node_noise`): without that the planet was see-through for
  seconds after loading and on landing. The loading screen (splash.rs) waits until the first view is
  loaded. se.log lists each pipeline that took over 0.5 s. A settings change that changes the view's
  pipelines (MSAA) still builds them on demand: the ground can be missing for seconds after it.
- Slowdowns over a session (the user plays for hours; test runs start fresh): measure with
  `.optimize/bench/session_explore.sh NAME ROUNDS [ENV=..]` (the same two views each round between trips;
  `session_table.py` prints the table) and read the counts `SE_PERF` writes per segment, `cpu_main`,
  `cpu_render`, `cpu_graph` (the render graph: recording, finishing and submitting command buffers) and
  `wgpu_peak`. wgpu sizes every pass's tracking by the most buffers and textures EVER alive at once (a
  peak never comes down, only a restart clears it), so one moment with too many GPU objects slows every
  later frame: this was the user's 80 to 40 fps. Keep the peak down: a limit on every cache of GPU
  objects (at most `PORT_LAND_NODES`, 4000, terrain nodes, the tile textures `TEXTURE_CACHE_MB`), a pool
  instead of a buffer per object, and no extra buffer per material (`PlanetMaterial` has one uniform; the
  biome's values live inside `Planet`; a second uniform doubled the ground's buffers). se.log says
  "wgpu: a new peak" every 2,000 buffers. Flat cloud shells (a material and a buffer each) are made only
  for nodes down to `FLAT_CLOUD_MAX_LEVEL` while the 3D clouds draw the clouds; when flat clouds are
  really drawn (`TerrainQuality::fine_flat_clouds`: clouds off, capture views) the drawn nodes get theirs
  on demand (`shell_jobs`). `SE_FLAT_FINE=1` makes them all for comparisons. When comparing GPU times
  alternate back to back (`ab_nodes.sh`): a capped or unfocused window clocks the card down, and rows with
  GPU 0.00 mean the window was minimised. A test's `agl M` stays on until `agl off`. A per-system timeline: build
  `CARGO_TARGET_DIR=target-trace cargo build --release -p se_engine --features bevy/trace_chrome` and run
  that exe with `TRACE_CHROME=FILE` (about 2 GB a minute; `perf NAME` marks a `perf_mark` span).
- Never change a Bevy material asset every frame (`Assets::get_mut`, `iter_mut`): Bevy then
  re-prepares every object using it. Per-frame values go in a storage buffer written in place: the
  star/galaxy sprites' camera values (`stars.rs` `SPRITE_CAMERA`) and terrain lighting (`terrain.rs`
  light pages). Write material assets only when a value really changes.
- After editing a shader, an older exe reading the new shaders can fail (bindings differ): rebuild.
- The catalog counts its changes (`Catalog::revision()`, and `edits()` for records changed in
  place); the nearest-star cache and the black-hole list rely on them. Change the catalog only
  through its own methods or `SeData::catalog_mut` / `catalog_get_mut`, and bump `edits` when code
  edits an existing record in place.
