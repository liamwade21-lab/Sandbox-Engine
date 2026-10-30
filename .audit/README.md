# SpaceEngine 0.991 -> Rust/Bevy port: feature-gap audit

Goal: list everything the port does not do, or does differently from, SpaceEngine 0.991, so it can
become a one-to-one copy. This is an AUDIT. Nobody changes the port in this phase.

## Places

- SpaceEngine install (READ ONLY, never write anything inside it): `D:\SteamLibrary\steamapps\common\SpaceEngine`
  - exe: `system/SpaceEngine.exe` (MSVC x64, no symbols). Config: `config/main-def.cfg` (all default
    settings, 1068 lines, sections listed below), `config/preset-*.cfg`, `config/def_*controller.cfg`.
  - data: `data/catalogs/Catalogs.pak`, `data/scripts/Scripts.pak`, `data/models/*`, `data/textures/*`,
    `data/sounds/{menu,spacecraft}`, `data/music/context.cfg`, `data/locale/eng-gui.cfg` (every UI string),
    `data/locale/eng-db.cfg` (object names/descriptions). `.pak` files are plain ZIPs: read them with
    `unzip -l`, `unzip -p PAK ENTRY` or Python `zipfile` to stdout (pipe into grep); never extract or copy SE
    files anywhere under the repo (not even `.audit/work/`) and never into the install.
  - Encrypted `data/shaders/Shaders.pak`: its 141 decrypted GLSL files are already in
    `re/se_shaders/` (reference only). These are SE's real shaders: the best evidence for rendering behaviour.
- Port: `C:\Users\liamw\Desktop\Space Engine Recomp`
  - `README.md` (sections "Build and run", "Controls", "Architecture", "Status") says what the port has.
  - `crates/se_core/src/*` (formats, catalogs, procgen, physics), `crates/se_engine/src/*` (Bevy app),
    shaders in `crates/se_engine/assets/shaders/*.wesl`.
  - `docs/*.md`, `docs/formats/*.md`: earlier reverse-engineering results (what was decoded, what was
    approximated). `docs/reverse_engineering.md` is the index.
- Earlier RE: `re/strings_ascii.txt`, `re/strings_utf16.txt` (all exe strings; strings under 4 chars are
  missing), `re/rtti_classes.txt` (241 C++ class names), `re/imports.json`, Ghidra project `re/ghidra_projects/se`.

## Tools

- Ghidra: a shared bridge is already running. Always pass the project:
  `ghidra <cmd> --projects-dir re/ghidra_projects --project se` (run from the repo root).
  Useful: `ghidra strings list --filter "value~Key"`, `ghidra x-ref to <addr>`, `ghidra decompile FUN_x`.
  Helpers (run from the repo root):
  - `python -I .audit/tools/gx.py Key [Key ...]` -> string address and the functions that reference it.
  - `python -I .audit/tools/dec.py FUN_x [FUN_y ...]` -> decompiles into `.audit/work/dec/FUN_x.c` (shared
    cache; reuse a file another agent already made).
  READ-ONLY use of Ghidra: never `stop`/`restart` the bridge, never rename, retype, set-signature, comment,
  patch, analyze or import. Other agents share the bridge; calls queue, so keep them targeted.
- Running the port (when unsure whether something works): use ONLY
  `bash .audit/tools/runport.sh OUT.png FRAMES "dev; commands" [extra args]`
  It serialises runs (one window at a time across all agents), runs `target/release/spaceengine-rs.exe`
  (already built and current; never run `cargo build`/`cargo run`, never touch `target/`), writes the log
  to `OUT.png.log`, prints the log's last lines. Put outputs in `.audit/work/<ticket>/`.
  Dev commands are in README "Build and run" (`goto NAME KM`, `select NAME`, `look NAME`, `surface ...`,
  `child N`, `nearest`, `rs RS ...`, `gal L B PC`, `date`, `rate`, `fov`, `shot FILE`, `wait N`, `click X Y`,
  `go`, ...). Use at least 200 frames before the final screenshot (shorter gives black captures); put
  `wait 1` between `shot`s. Look at the PNGs with the Read tool. Env vars like `RUST_LOG=info` pass through.
  Keep runs few and short (each holds the lock): batch several views into one run with `shot`.
- Do NOT start SpaceEngine itself (taking over the user's screen needs the user's approval). Evidence about
  SE comes from its files and code.

## Rules

- Do not edit anything outside `.audit/work/<your ticket>/` and `.audit/gaps/<your ticket>.txt`.
  No changes to `crates/`, `docs/`, `README.md`, `re/`, `target/`, or the SE install.
- Out of scope, do not report: VR, the editors (planet/star/object/ship editors, `Edit*` tools),
  Steam Workshop / online services / DRM, the `LaunchPad.exe` launcher, OpenGL-specific options
  (the port is Vulkan only: report the visible effect of an option, not its GL mechanics).
- Report gaps only (missing, partial, or behaving differently). Do not list things that match.
- A gap needs SE evidence: a shader file+line, a config key, a locale string, an exe string or decompiled
  function, a catalog key, a script command. Then check the port for it (grep `crates/`, read the code,
  run it if still unsure). "Not found by grep" alone is weak: search for synonyms and the README too.
- Keep features at a useful grain: one line per thing a developer would fix as a unit (not one line per
  config key when ten keys belong to one missing window; but not "UI is different" either).

## Output

Write your gaps to `.audit/gaps/<ticket>.txt`, one per line, and return exactly the same lines as your final
answer (nothing else except one first line `TICKET <id>: <n> gaps`). Line format:

`<ticket>-<nnn> | visibility | feature | SE evidence | port location or "none" | missing/partial/differs | confidence`

- visibility 1-5: 5 = obvious in the first minutes of normal play with default settings; 4 = seen in common
  activities (visiting planets, stars, galaxies, using the main panels); 3 = seen when using a specific
  common feature or window; 2 = niche setting, rare object or rare situation; 1 = obscure.
- SE evidence: short and checkable, e.g. `star_sprite.glsl:40-70 glare ring`, `main-def.cfg StarSizeMag`,
  `eng-gui.cfg "Show constellations"`, `FUN_1402a5c50`, `Catalogs.pak stars/x.sc key Teff`.
- port location: `crates/se_engine/src/stars.rs:120` or `none`.
- confidence: high (seen in SE code/data AND checked in the port code or a run), medium, low.
- Say what differs in the feature field, concretely (e.g. "star glare: SE draws 4-spike diffraction cross
  scaled by StarGlareSize; port draws a round glow only").

## main-def.cfg sections (line numbers)

Memory 6, Vendor 26, Display 47, RenderingQuality 62, DepthBuffer 76, PlanetRendering 85,
AtmosphereRendering 123, RingsRendering 138, PointRendering 152, WarpRendering 184, PostEffects 196,
Lighting 240, TextureRendering 297, Projection 351, VR 387, Sound 424, Physics 439, Export 453, Loader 475,
Log 501, Interface 525, Units 578, Toolbars 615, SolarSystemBrowser 625, SolarSystemChart 636,
UniverseMap 651, ObjectFilters 664, OverlayFilters 688, DebugOverlay 754, Controls 793, VelocityPresets 823,
Ephemerides 854, SolarSystemGenerator 863, LifeGenerator 896, PlanetGenerator 933, StarGenerator 966,
ClusterGenerator 978, NebulaGenerator 1015, GalaxyGenerator 1048, Engine 1059.
