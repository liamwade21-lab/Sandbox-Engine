# Sandbox Engine — Architecture (the Sidewinder's original assets)

**Updated:** 2026-11-24 (A10 and its addendum: Elite's ship UI; A11: the parked ship is a sandbox prop). Executors follow this exactly; only the orchestrator changes it.

**Scope of this file:** finishing the Sidewinder mod: Elite Dangerous's original assets (hull, textures and materials,
cockpit, chair, animation, ship audio) in the game, the ship's systems, and the flight's remaining rules and numbers.
Everything else about the project is in `CLAUDE.md`, and the mod's own structure is in `docs/spec-mods-sidewinder.md`
("Architecture", the Constraints table); both still apply in full. **Since 2026-11-16 one session (the
`/project-coordinator` run) owns all of the mod's files; A8 replaces A4's ownership table.** Designs A5 to A8 are this run's.

## Overview

Three stages, each its own tool, with a folder between them. Nothing skips a stage.

```
Elite's install (read only)          re/elite/frea_export/           crates/se_engine/assets/mods/sidewinder/
  Win64/0?/<40 hex>  FREA files  -->   catalog + the Sidewinder's  -->  the mod's own formats (.edm, textures,
  + re/elite/keys/frea_key.txt         decrypted payloads, as they      sounds, mod.cfg, SOURCES.txt)
                                       come out (nothing converted)
        1. EXPORT (out of git)              2. CONVERT (in git, test only)         3. RUN (the game)
        re/elite/tools/frea/                sidewinder/convert.rs                  sidewinder/*.rs
```

## Requirements and constraints

- **Source scope:** the Sidewinder's models, textures, cockpit, animations, audio and associated data.
- **Offline inputs:** use an authorized key (default path `re/elite/keys/frea_key.txt`, override with `ELITE_FREA_KEY`)
  or a decrypted export. Export tooling and format notes are in `re/elite/tools/frea/`.
- **The key's value never leaves its file:** not in code, logs, test output, reports, briefs, `PROGRESS.md`, the spec,
  anything git holds, the mod's assets or a release zip. Tools read it from the path at run time.
- **Elite's install is read only.** Open its files for reading only; write nothing under `D:\SteamLibrary`.
- **Decrypted data at rest is the Sidewinder's only.** Finding its files means opening every FREA file (their names say
  nothing), so the catalog pass decrypts in memory and keeps one catalog row per file; only files identified as the
  Sidewinder's are written to `re/elite/frea_export/sidewinder/`.
- **Offline decryption.** The export tool lives in `re/elite/tools/frea/` (out of git). `se_core` parses the exported
  payloads and the running mod loads converted assets.
- **C4:** the running mod reads only `assets/mods/sidewinder`. The install and the export folder are read only by the
  export tool and by `#[cfg(test)]` code.
- **The mod's files rules** (`CLAUDE.md`): every file a `Need` in `mod.cfg`, nothing built into the exe, no file over
  90 MB, written only by `copy_sidewinder`, never edited by hand. Keep each file's line endings.
- **What must not break:** the mod with its current files' behaviour (spawn, board, flight, HUD, hyperspace), the mod
  off (bit-identical ground), `cargo test -p se_core`, `cargo test -p se_engine --release`, `cargo test --release -p
  elite-sim`.

## Repository layout (what this work touches)

| Path | In git | What lives there |
|---|---|---|
| `re/elite/keys/` | no | the key, Frontier's reply, a README |
| `re/elite/tools/frea/` | no | the export tool (stage 1) and its notes on the format |
| `re/elite/frea_export/` | no | `catalog.csv` (every FREA file, one row), `sidewinder/` (its decrypted payloads + `manifest.json`) |
| `crates/se_core/src/elite.rs` | yes | readers for Elite's *decrypted or open* containers (meshes, textures, sound packs) |
| `crates/se_core/src/elite_asset.rs` | yes | the mod's own formats (`.edm`, `.binds`) |
| `crates/se_core/src/elite_ship.rs` | yes | the ship's data files' shapes (serde): materials, assembly, chair, sounds (A5) |
| `crates/se_core/src/elite_rig.rs`, `elite_chair.rs` | yes | Elite's animation rules as pure code: rigs, curves, state machines; chair clips and motion graph (A5) |
| `crates/se_engine/src/sidewinder/convert/` | yes | `copy_sidewinder`, the converter (stage 2, `#[cfg(test)]`): `mod.rs` and one file per kind (A5) |
| `crates/se_engine/src/sidewinder/draw.rs` | yes | exterior, cockpit, materials, lights |
| `crates/se_engine/src/sidewinder/parts.rs`, `chair.rs` | yes | the moving parts (gear, doors, hardpoints, vents) and the pilot chair in the game (A6) |
| `crates/se_engine/src/sidewinder/sound.rs` | yes | the ship's sounds |
| `crates/se_engine/assets/mods/sidewinder/` | yes | the mod's files (stage 2's output) |
| `crates/se_engine/assets/shaders/sidewinder_*.wesl` | yes | the mod's shaders |

## Modules

| Module | Its one job | Owns | May depend on |
|---|---|---|---|
| Export tool | FREA file + key -> decrypted payload; the catalog; the Sidewinder's export | `re/elite/tools/frea/`, `re/elite/frea_export/` | the install (read), the key file. Nothing in `crates/` |
| `se_core::elite` | parse Elite's container and payload formats from bytes | `crates/se_core/src/elite.rs` | std, the crates `se_core` already uses. No Bevy, no paths, no key |
| `se_core::elite_asset` | the mod's file formats, read and write | `crates/se_core/src/elite_asset.rs` | std |
| `se_core::elite_ship` | the shapes of the ship's JSON files, read and write (serde) | `crates/se_core/src/elite_ship.rs` | std, serde, serde_json |
| `se_core::elite_rig`, `elite_chair` | Elite's animation rules: pose a rig, run curves and state machines; sample, blend and choose chair clips | `crates/se_core/src/elite_rig.rs`, `elite_chair.rs` | `elite_ship`, glam; `elite_chair` also `elite_asset` (the chair's skeleton from its `.edm`). No Bevy, no files |
| `se_core::elite_sound` | the shape of `sounds.json` and a pure mixer that does what Wwise does with it (events in, voice updates out) | `crates/se_core/src/elite_sound/` | `serde`, `serde_json`, std. No Bevy, no audio device, no files |
| Converter | decoded export -> the mod's files; reads everything back | `sidewinder/convert/` | `se_core::elite*`; the export folder and install (test only) |
| Draw | draw the hull, cockpit and chair with their materials and lights | `sidewinder/draw.rs`, the `sidewinder_*.wesl` shaders | `se_core::elite*`, Bevy, the sandbox's light record |
| Parts, Chair | drive `elite_rig` / `elite_chair` from the ship's state; hand `draw` the transforms | `sidewinder/parts.rs`, `chair.rs` | `se_core::elite_rig`, `elite_chair`, `Pilot`'s public state |
| Sound | play the ship's sounds from flight state | `sidewinder/sound.rs` | `se_core::elite_ship`, `Pilot`'s and `Parts`' public state, Bevy audio |
| Flight | the ship's rules and numbers | `crates/elite-sim/` | glam only |

Dependency direction: export tool -> (files) -> converter -> (files) -> game. `se_engine` depends on `se_core`, never
the other way. `elite-sim` knows nothing of assets. No module reads another stage's input. Inside the mod: `pilot`
(flight state) knows nothing of `draw`, `parts`, `chair` or `sound`; those read `Pilot` and never write it, except
`parts`, which reports the gear's and hardpoints' real state back through one small public struct that `pilot` reads.

## Interfaces

- **Key:** a text file of hex digits; whitespace and line breaks are ignored when it is read (512 digits, 256 bytes).
- **Export folder** (the contract between stages 1 and 2):
  - `re/elite/frea_export/catalog.csv`: one row per FREA file: `name,folder,size,inner_kind,unpacked_size,payload_sha1,
    notes`. No payload bytes.
  - `re/elite/frea_export/sidewinder/<40 hex>.bin`: a decrypted payload exactly as it comes out (after decryption and
    the container's own decompression, nothing converted), named by its source file.
  - `re/elite/frea_export/sidewinder/manifest.json`: for each exported file its role (`hull`, `hull_lod`, `cockpit`,
    `texture`, `material`, `animation`, `sound_bank`, ...), what it belongs to and the evidence it is the Sidewinder's.
- **Converter inputs:** `ELITE_PATH` (the install's product folder, as today) and `ELITE_EXPORT` (default
  `re/elite/frea_export/sidewinder`). Without the export folder `copy_sidewinder` fails with a message naming it; there
  is no fallback (the stand-in models were removed on 2026-11-18).
- **Export tool commands** and the FREA layout: see A1 below.

## Standards

- Every rule or number comes from the files, the exe's code or a measurement; nothing tuned by feel. A recreated part is
  labelled a stand-in with its grade (the spec's grades) in `SOURCES.txt`.
- `SOURCES.txt` in the mod's folder lists each shipped file's source file name(s) and grade. It never holds the key.
- The converter reads back everything it wrote and fails if anything is missing or differs.
- Errors: a damaged or unexpected file is an `Err` with the file's name, never a panic in the game.
- Tests for parsers sit beside them and run without the install (small synthetic bytes); tests that read the install or
  the export are `#[ignore]`.
- Quiet output when running builds and tests: failures and the last ~30 lines only.

## Technical designs

- **A1 FREA export (T1):** see `re/elite/tools/frea/FORMAT.md` for the format and reader commands.
- **A2 Finding the Sidewinder (T2, done 2026-11-15):** the payloads are Cobra-engine OVL containers; resources are
  found by name in the catalog (`catalog_names.tsv`), their streamed data in open files matched to the OVL's archive
  records. Formats: `re/elite/notes/sidewinder_assets.md`. The decoded export (the contract for T3):
  `re/elite/frea_export/sidewinder/decoded/` (`README.md` first: the exact vertex decode from Elite's shaders, the spaces
  and the conversion to the mod's; `manifest.json`, `models/*.json` and their chunk `.bin` files, `materials.json`,
  `textures.json`, `textures/*.dds`, `shaders/`, `vertex_test_vectors.json`, `animation/`: rigs, state machines, sound
  cues and `kinpose.py`, formats in `re/elite/notes/sidewinder_animation.md`) and `re/elite/frea_export/sidewinder_audio/`
  (`manifest.json`, `ogg/`; the gear's sounds are roles `gear_*`). The `.edm` keeps Elite's raw 40-byte vertices, so the decode belongs in the mod's loader
  (or the converter), checked against `vertex_test_vectors.json` (`se_core::elite::Vertex::decode` is that decode).
- **A3 The mod's own formats (decided 2026-11-15, from a map of the code as it is):**
  - **Models stay `.edm`** (`SWEDM001`; since A5 version 2, and since 2026-11-18 the reader knows only version 2: the
    stand-ins that used version 1 are gone). It already holds Elite's raw 40-byte vertices (position, normal,
    tangent, uv, uv2, colour, bone indices and weights), materials with texture slots (`(slot, file)`: `diffuse`,
    `normal`, `mask`, `glow`), flags (GLASS 1, EMISSIVE 2, TWO_SIDED 4) and animations whose keys carry rotation. Add a
    field only if the originals need one; then bump the version and change `write_edm`, `parse_edm` and
    `edm_round_trips` together.
  - **Textures are `tex/NAME.dds`** in the mod's folder, read as the hero's are (`se_core::dds::parse_dds`, then
    `mr_assets::dds_image`), block-compressed data kept as Elite has it. `parse_dds` knows BC1, 2, 3 and 5 today: a
    format beyond those is added there, never re-encoded with loss.
  - **Sounds are `sfx/NAME.ogg`** (Ogg Vorbis), loaded through `mods::check_sound` as `skate/sound.rs` does, played by a
    new `sidewinder/sound.rs`.
  - **Drawing today:** one Bevy mesh per draw (position, normal, UV_0), `PropMaterial` with `sandbox_prop.wesl`, a 1x1
    white texture and the material's tint; GLASS draws are skipped; tangents, uv2, vertex colour and EMISSIVE are
    unused. T4 gives the ship its own material and shader (`sidewinder_ship.wesl`) lit by the same light record
    (`PropLight`); `PropMaterial` and `sandbox_prop.wesl` stay as they are.
  - **Known loose ends for T3:** `se_core::elite` only detects texture and sound payloads (no readers yet); the
    converter labels the hull's source `#0/0` although it takes the largest part.
- **A4 Ship systems and audio development:**
  - **Who owns which files in a wave** (two writers at most, never the same file): the *systems* writer owns
    `crates/elite-sim/src/*`, `sidewinder/pilot.rs`, `hud.rs`, `panels.rs`, `seat.rs`, `mod.rs`; the *sound* writer owns
    `sidewinder/sound.rs`, `sidewinder/sounds/`, `sidewinder/convert.rs` and the mod's `sfx/` files and `mod.cfg`.
    `mod.rs` already calls `sound::plugin(app)`: the sound writer never needs `mod.rs`.
  - **Sounds (the spec's Phase 3) with our own stand-ins.** `sidewinder/sound.rs` is the whole mixer: loops (engine,
    thrusters, boost, the cockpit's hum, the drive's charge, supercruise) and one-shots (gear, scoop, hardpoints, flight
    assist, lasers, alarms, panel clicks). It reads `Pilot`'s and `Seat`'s public state only and finds events by comparing
    with the frame before; it changes nothing in them. It plays only while seated. The sound files are **made by us**
    (`sidewinder/sounds/make_sounds.py`: numpy and ffmpeg, no recordings, nothing of Elite's), grade E, each under the
    file name its real counterpart would take (`sfx/NAME.ogg`), so a real one can replace it later with no code change.
    `copy_sidewinder` copies them into the mod's folder and writes their `Need`s. **Not built:** the cockpit voice (3.3:
    its lines are Frontier's recordings).
  - **Stand-in numbers stay labelled** (`stand_in` modules, grade E, the reason in a comment) until a measuring session
    replaces them; a rule never gets a number tuned by feel.
  - **Each task's report carries its Build log entry** (12 lines at most, in the spec's own style); the orchestrator adds
    it to `docs/spec-mods-sidewinder.md`, so two writers never edit that file.
  - **Superseded 2026-11-16 where A7 and A8 differ:** the ship's real sounds are decoded, so no stand-in sounds are
    made and the cockpit voice is built (A7); file ownership is per task (A8).
- **A5 The ship's files (T3; the contract between the converter and the game):**
  - **Source:** only `re/elite/frea_export/sidewinder/decoded/` (its `README.md` is the reference) and
    `re/elite/frea_export/sidewinder_audio/`. Nothing is decrypted again; the converter never needs the key.
  - **Which models:** what the ship item proves the game draws: hull `sidewinder` with every part its rig hangs, the
    three gear rigs' parts, the drive bolt-ons, cockpit `sidewinder_cockpit` with `sidewinder_screen` (and its damaged
    twin and the 20 shards), chair `cockpit_chair` and `ext_cockpit_chair`, `sidewinder_schematic`, `sidewinder_shield`,
    the `cockpit_ui_hightlight_*` overlays. Never the `pg_` copies. **LOD0 only** (Elite's LOD switch rule is not known;
    `SOURCES.txt` says so).
  - **`models/NAME.edm`** (`.edm` version 2): Elite's 40-byte vertices and u16 indices byte for byte (grade A,
    hash-checked against the export), each draw naming its material by Elite's name, bones with parent, local
    translation / rotation / scale and the stored inverse bind. Vertices stay raw in the file; the exact decode
    (the README's, float32) is `se_core::elite::Vertex`, tested against `vertex_test_vectors.json`. Conversion to the
    mod's space (x, y, -z; winding; tangent sign) happens at load, in one function.
  - **`tex/NAME.dds`**: Elite's texture levels as decoded, bytes unchanged (BC1/3/4/5/7, B8G8R8A8). `parse_dds` learns
    BC7 (and anything else the set needs); nothing is re-encoded.
  - **JSON for structured data** (`se_core::elite_ship`, serde; `se_core` gains `serde` and `serde_json` from the
    workspace): `materials.json` (one table for every model: shader name, texture slots or constant colours, every
    attribute), `ship.json` (the assembly: models and where they hang, the rigs' node trees and motions, the state
    machines with typed activities including lights and sound cues, the paint job record, named anchor bones),
    `chair/clips.json` + `chair/clips.bin` + `chair/graph.json`, `sounds.json`. Fields keep Elite's own names and units
    (Elite's model space, seconds, radians or degrees as stored); nothing is pre-chewed that a rule later needs.
  - **Pure rules in `se_core`:** `elite_rig` is `kinpose.py` (pose, `apply_motion`, `eval_curve`, resume and dominant
    curve rules) plus the state machine runner (cheapest route, `wait_for_activities`, typed activities raising
    events); `elite_chair` is `manipose.py` and `blend_ref.py` plus the motion graph's evaluation. Each is tested
    against vectors made by running the reference Python, kept as small JSON in `crates/se_core/tests/data/elite/`.
  - **Converter layout:** `sidewinder/convert/mod.rs` (`copy_sidewinder`: runs each kind, writes `mod.cfg` and
    `SOURCES.txt` from what they report, reads everything back) and `models.rs`, `assembly.rs`, `chair.rs`,
    `sound.rs`, each `pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String>`. A kind not built
    yet is an empty function, so each later task edits only its own file.
- **A6 Drawing and moving the ship (T4):**
  - **Files, one job each:** `assets.rs` (reads the mod's files, decodes the models, owns the conversion between
    Elite's space and the ship's, builds the solid shape, feet, eye and gun points from bones and rig), `parts.rs`
    (which pose the moving parts have now), `draw.rs` (meshes, placement, what shows), `material.rs` (`ShipMaterial`
    and its shader families), `lights.rs` (the ship's lights), `chair.rs`, `sound.rs`. `mod.rs` keeps the plugin, the
    requests and the parked ship.
  - **Ship space** (what `Parked`, the solids, the seat and the flight use): X right, Y up, -Z the nose, the origin on
    the ground under Elite's model origin with the gear Down. It is Elite's model space mirrored in z and lifted by the
    rest height the rig gives (4.718 m, computed at load from the posed gear, never typed in). `assets.rs` has the one
    pair of functions that convert points and matrices; nothing else converts.
  - **The drive bolt-ons** (`pg_sidewinder_extendedengine*`) are the arena's by the evidence at rest: not shipped.
  - **What shows when** (decided 2026-11-16): the cockpit model carries its own copy of the hull shell as seen from
    the seat, and the outside chair is a low-detail twin of the cockpit's. So from the seat only the cockpit rig's
    models and `cockpit_chair` are drawn; from outside only the hull rig's, the gear rigs' and `ext_cockpit_chair`.
    The two sets are never drawn together.
  - **Materials:** a `ShipMaterial` with `sidewinder_ship.wesl` (variants by Elite shader family) works out base colour,
    normal, roughness, metalness and emissive exactly as the decoded shader disassembly does (`decoded/shaders/`:
    tiles through the mask, the paint job's keys and hard thresholds, decal groups, cockpit colour override), then
    lights them with the sandbox's light record (`PropLight`) and the ship's own lights. The lit maths (BRDF,
    inverse-square with the windowed falloff, spot cone, IES lookup) is ported from Elite's lit shaders and the notes.
    `PropMaterial` and `sandbox_prop.wesl` stay as they are.
  - **The known gap:** Elite scales its lights and glows by engine values that were never found. Each has ONE labelled
    stand-in in a small buffer `ShipScene` (made in `material.rs`, written by `lights.rs`): `scene_ev` (the lighting
    manager's scene value), `self_illum_scale` (`gGlobalSelfIllumScale`), `emissive_multiplier`
    (`gEmissiveMultiplier`), `luminance` (the auto-exposure texel the emissive rescaling reads). Each emissive family
    reads exactly the global its disassembly reads. Nothing else about the lights is invented.
    - **The lamps' exposure factor is applied in the shader (since 2026-11-18)**, not on the CPU: a lamp that carries
      one is multiplied by 2^(clamp(scene_ev + bias, min, max) - 3) with its own bias and clamps, which sit per slot in
      `ShipScene` (`lamps`, the lights buffer's order). The reason: the scene value must follow the exposure the frame
      is really drawn with, and in photo mode 2 the automatic exposure's adapted compensation lives only on the GPU.
      `sidewinder/exposure.rs` copies that state into the ship's own small buffer (`ShipScene::adapt`, binding 17)
      with a compute pass before the opaque pass: no readback, one frame behind while adapting.
    - The stand-ins' values: `ELITE_UNIT` 1/256, Elite's light units to ours (E: a round number that two
      measurements of the green nav light against the user's picture from above bracket); 
      `COCKPIT_LIGHTING_BRIGHTNESS` 1/11.4 on the cockpit's lamps (D: measured by stepping the value in one run and
      comparing lamp-lit regions with the user's seat picture, good to about a factor of two because the regions sit
      in the tone curves' toe; it stands in for the lighting brightness the exe study did not find); the HUD colour's
      hue for the GUI panel lamps (D, from the user's picture).
    - Per-frame values a material needs go into small buffers written in place, never into the material: the
      lights and `ShipScene`, the heat's glow (`HeatGlow`), the canopy's crack amount (`ShipGlass`).
  - **The seat view:** while seated the camera's vertical field of view is 56.249 degrees (the user's Elite setting,
    grade B), given back on leaving; cockpit, glass and chair are drawn in the nearest depth slice, as the weapons'
    view models are, so the port's altitude-driven near plane never cuts them.
  - **The ship's lights** are one storage buffer written in place when a value changes (never a material edit). The
    headlight reaches the ground, trees and props through `ground_light.wesl` with a record that is zero when off, so
    the ground stays bit-identical with the mod off (`.optimize/bench/pixel_scenes.sh`).
  - **The lights buffer (the contract between `material.rs` / the shader, which read it, and `lights.rs`, which fills
    it):** `ShipLights { count: u32, _pad: vec3<u32>, lights: array<ShipLight, 96> }`, each `ShipLight` 64 bytes, all
    in ship space, metres, linear colour: `pos_range` (xyz position, w range), `colour` (rgb colour x level x the
    stand-in units, WITHOUT the exposure factor, which the shader applies; w source size), `dir_cos` (xyz spot axis, w
    cos of the half cone; w = -2 marks a point light), `spot` (x cos of the inner half angle, y K = pi / cone for the
    IES lookup, z 1 if the IES profile applies, w the specular scale). Each material's params say which ONE contiguous
    range of the list lights it (`first`, `count`). The slot order makes Elite's rule a pair of ranges: the hull's
    unflagged lights 0..27, the vent lights 27..47, the cockpit's flagged lamps 47..82, the red warning lamp 82..84.
    Elite's light gather gives a view with flag bit 3 only the lights whose data byte +0x5a is set (the vent lights
    and the cockpit's lamps; `FUN_1435b9020`): the seat's materials take 27..82, the outside's 0..47 (the outside
    taking only the hull's lights is ours: the unfiltered view's layer mask was not traced). A light whose state is
    not its machine's current state gives no light at once (Elite's Stop unsubscribes it, `FUN_1435bc330`); its slot
    stays (nothing reorders); the shader skips a light whose colour is zero or whose range the pixel is outside.
    (In WGSL the `vec3<u32>` pad puts the lights at byte 32: `ShipLights::write` in `material.rs` is the one
    writer of the layout.) A second small buffer, `ShipPose`, gives the shader the ship's pose this frame (the
    lights are in ship space); it is written only while a light is on.
  - **The hull's wear and dirt:** Elite replaces three of the painted and bare-metal materials' attributes per ship
    at run time (`pDirtLevelScale`, `pLayerMaskCutoff`, `pDirtDiffuseOverride`) through an override object; the mod
    does the same through one override struct in `material.rs`, never by editing the materials' data.
  - **Shader files:** `sidewinder_ship.wesl` (the vertex stage, the slice, the family dispatch), one file per group of
    families for the surface functions (`sidewinder_surface_hull.wesl`, `sidewinder_surface_cockpit.wesl`,
    `sidewinder_glass.wesl`), `sidewinder_lit.wesl` (Elite's lit maths: the BRDF, the sun and sky from the sandbox's
    light record, the loop over the lights buffer). Each surface function names the disassembly file it ports.
  - **Moving parts:** `parts.rs` runs `elite_rig` machines from requests (gear down / up, hardpoints, heat, lights)
    and gives `draw` each part's matrix; the gear's real state and feet come from the rig, replacing the recreated
    gear's numbers (`GEAR_M`, `GEAR_SECONDS`...). `chair.rs` feeds `elite_chair` the throttle, stick, yaw, trigger and
    buffets and writes a bone palette, skinned as the sandbox's view models are.
    - One `Runner` per machine lives in `Parts`. Each frame: requests (what the ship's state asks of each machine:
      `machines.rs`, a table marked row by row as the exe's, the notes' or ours), then every runner steps by real
      seconds (the doors' machines before the legs'), then the events are acted on, then the pose if a value changed.
      A spawned ship is settled silently (run forward, events dropped), never animated into place.
    - **One machine waiting for another (2026-11-18):** a gear leg waits for its bay doors. In Elite the leg's
      `DoorClientRequest` activity is unfinished while its door is busy (`0x1443aa5d0`), and a waiting transition
      leaves only when every activity of the state is finished. The pure crate knows no other machine, so the
      interface is one closure: `Runner::step_with(dt, values, door_busy: &dyn Fn([u16; 2]) -> bool)`;
      `Runner::step` means "no door is ever busy". The game answers the closure from the door's machine.
    - The gear's state for everything else (`pilot.rs`, the sound, the lights) is `Parts::gear()`: the exe's own two
      tests, "deployed" (gear state 2 or 4) and "moving" (1 or 3), and the raw state per leg (the tests themselves are
      pure functions in `se_core::elite_rig`). The flight's rules read different things, each as Elite's code does:
      the speed limit and the frame shift drive's refusal read the gear's REQUEST (the pilot's command); the landed
      state needs "deployed"; the legs touch the ground from state 1 on (`Parts::feet()`: each leg's foot at its
      current pose), so a ship coming down on moving legs is held by them.
    - **Who reads whom (2026-11-18, after a review found two loops):** `facts.rs` holds `ShipFacts`, the ship's facts
      this frame (power, silent running, the gear's command, the canopy, the drive's phase, the heat and its
      dissipation values, the open side panels, the ship's damage, the pilot's controls for the chair, the tests'
      overrides), written once a frame from `Pilot` and `Seat` before anything else of
      the mod runs. `machines.rs` / `parts.rs` read `ShipFacts`; `lights.rs` reads `ShipFacts` and `Parts` (the states, the gear,
      and the parts' values and matrices to place the lamps at their bones);
      the sound reads `ShipFacts`, `Pilot` and `Seat` (the flight's state and the head's turn), `Parts::gear()`,
      `Parts::events()` (every cue of the machines with its machine's index and bones, and the audio states, for one
      frame) and the parts' values, matrices and revision (to place the emitters). Nothing reads the other way: the
      parts never call the lights or the sound (since 48325b9 no call to the sound is left in `parts.rs`). Rules
      that are Elite's and not this game's live in the pure
      crates: the machines' in `se_core::elite_rig`, the heat's dissipation in `elite_sim::heat`.
    - Requests are made when what the ship wants of a machine CHANGES (and once when the ship is made), as Elite's
      code does on a power change, never every frame.
  - **The last three drawing tasks (decided 2026-11-17, from a map of the code: the ship is one Bevy material; only
    the chair is skinned; the engine has no shadow map and no local light on the ground; the clouds' passes run
    after Bevy's transparent pass; the tunnel is an opaque egui layer):**
    - **T4e-1, the ship's shadow (`sidewinder/shadow.rs`, new; a pass of the mod's own, running only while a ship
      exists and the star's light on it is not 0).** One orthographic map of the ship from the star, in ship space,
      made by the mod's own pipeline from the mod's own copy of the casters' triangles (one vertex buffer made at
      spawn, a part index a vertex, the parts' matrices from a buffer written in place; never Bevy's mesh buffers).
      Two depths a texel: the nearest caster and the furthest. Casters: every opaque family; never glass, overlays
      or stamps. (a) The ship's own shader multiplies the star's term by the map's answer (`sidewinder_lit.wesl`:
      Elite's lit maths has the star's shadow flag; if the ported shader source holds Elite's own filter, use it,
      else a plain 3x3 comparison filter, marked ours). (b) Everything else (the ground, props, buildings, trees)
      gets the shadow from one pass before the clouds that works from the depth buffer, as the mod's craters do: a
      pixel further from the star than the map's FURTHEST depth there is behind the ship and loses the star's
      direct share (the same share the hero's shadow uses, from the light record at the ship); a pixel between the
      two depths is the ship itself and is left alone (its own shader has done it); the seat's depth slice is left
      alone. Known and accepted: a thing standing between two parts of the ship along the star's ray (a prop under
      a wing in front of a gear leg) gets no shadow there.
      **As built (d4e7499) and what its review changed (2026-11-17):** the filter is Elite's own, found in its lit
      shader and ported line for line (moment shadow mapping, a 4x4 B-spline; the map holds moments, its write the
      inverse of Elite's read), plus a depth map of the furthest caster. (b) as a pass over the picture is WRONG and
      is replaced: a picture cannot tell how a pixel used the star's light (grass and trees are lit once an instance,
      as if facing up; a wall facing away has no star light to lose and went black; the walker's weapon was skipped).
      **New decision for (b):** the ship's shadow on the world is a term of each receiver's OWN lighting, beside the
      clouds' shadow on the star's light: one record `shade` appended to `CloudShadow` after `beam` (the matrix from
      the receiver's space to the map, zeros with no ship or no star light), the furthest-caster map as one texture
      the ground's material, the scatter's light pass and the hero / prop light pass each bind (a 1 x 1 "no shadow"
      texture until a ship gives one), one function in `ground_light.wesl`. The ground takes it per pixel; grass,
      trees and buildings once an instance; the walker's weapon, the hero and the props at the light pass's one
      point (the player's place: a prop elsewhere follows the player's shadow, a known limit of that record). With
      no ship everything stays bit for bit. The mod writes a resource (`ShipShade`, beside `ShipBeam`) and owns the
      map; the pass over the picture is deleted. **Corrected by its writer's finding (2026-11-18):** in this Bevy a
      material's bind group is NOT rebuilt when an image it binds changes, so the texture cannot be swapped in at a
      spawn. The engine therefore owns ONE moments texture from start-up, sized for the outside set's three maps
      (about 22 MB, paid by every player; nothing is read from it without a ship), the mod's passes draw into it,
      and the ship's own materials read the outside set from it too; the seat's cockpit set stays in a texture of
      the mod's own, made at the first spawn and kept. The light pass writes the star's light twice: without the
      ship's shadow for the ship's own materials (they keep `star_shadow`), with it for the props, the hero, the
      weapon and the skater. The shadow's softness follows Elite's own shadow settings (the
      cascades' sizes in its plain graphics configuration file at the user's quality), not our choice of map.
      **The softness, decided from the study (`re/elite/notes/sidewinder_shadow.md`, 2026-11-17):** Elite fits each
      cascade to a slice of the view (the user's profile: 1,536 texels a cascade; cascades end at 5, 80.8, 739.2 and
      6,500 m; crossfades 0.15 and 0.2), makes the moments at HALF the depth map's resolution, blurs them (a
      separable Gaussian, radius 2 for the first cascade, 1 for the others) and only then reads them through the
      B-spline. A pixel's shadow therefore has the texel of the cascade its distance from the camera falls in
      (0.023 m under 5 m, 0.376 m to 80.8 m, 3.44 m to 739 m at 3440x1440; the study's formula gives them for any
      window). The mod keeps maps fitted to the ship, but one for each of Elite's cascades, each with Elite's texel
      for that cascade (its size = the ship's box / that texel: about 1,120, 69 and 8 texels), each made as Elite
      makes it (depth at twice the size, moments at half, the blur), and the lookup picks the map as Elite's own lit
      shader picks a cascade (corrected after the review, 2026-11-17): the FIRST cascade whose light-space box
      contains the point, faded by the distance to that box's edge; not by the distance from the camera (the first
      box is centred 5 m ahead and about 9 m wide either side, so a point 12 m straight ahead is still in it). The
      maps are sized from the projection the frame is really drawn with (the seat's own field of view when seated). The seat's cockpit set needs the first cascade only. The
      filter's inputs are Elite's: moment bias 3e-5, light-bleeding cutoff 0.3 (read in the exe). The world's
      receivers (T4e-1b) read the same moment maps through the same filter (the filter's maths moves to an
      engine-level shader file both sides import), not a depth map with a filter of ours.
    - **T4e-2, the headlights on the ground.** The ground needs its own colour to be lit, so the lamp is a term of
      the ground's own lighting, not a pass over the picture. One record of the lamps (their places relative to the
      camera, directions, cones, range, colour, and the beam's profile as a table if the IES profile is the same
      round its axis, else a texture) is appended to `CloudShadow` AFTER `hero_end` (the record every ground,
      scatter, hero and prop shader already has; `ground_light.wesl`, `clouds.wesl`, `volumetric_clouds.rs` declare
      it and must change together). The mod writes a resource (as `HeroSoil`), `volumetric_clouds.rs` copies it into
      the record only when it changes; with no ship or the lamps off the record is zeros and one uniform branch
      skips the term, so every picture without it stays bit for bit (checked with `.optimize/bench/pixel_scenes.sh`).
      The lamp's maths is ONE function (`ground_light.wesl`), the same rule as the ship's own lamps
      (`sidewinder_lit.wesl` `ship_lamps`): falloff, cone and profile from Elite's data, the same exposure factor
      (the frame's real exposure reaches the record the way `exposure.rs` already copies it for the ship). Users:
      `planet.wesl` `lit_ground` (per pixel, on the ground's normal) and the scatter's per-instance light (as if
      facing the lamp, ours). NOT the light pass of the hero and the props (decided after it was built,
      2026-11-17): the ship's own materials read that same light record and would be lit by their own lamps twice,
      and the record is one point for every prop; a prop in the beam stays dark, a known difference. As built
      (afb1ef1): four lamps (two 500 m spots, two 5 m lamps at their housings); the profile is a table of 1,024
      values read by each caller through its own binding; the beam's record is 4,432 bytes.
    - **T4e-3, the tunnel behind the cockpit.** The hyperspace tunnel stops being an egui layer over everything: it
      is drawn in the 3D scene behind the cockpit (a shell round the camera in the main pass, its look the same
      stand-in as today, ours), so the cockpit, the chair, the glass and the HUD stay in front of it and nothing of
      the universe shows through it. The canopy's glass drawing after the clouds (today the clouds are laid over
      it, so its reflections, dirt and cracks fade where a cloud is behind) is done only if it can be done without
      touching how the engine's passes treat views: the one design allowed is a second camera that draws only the
      glass (and the cockpit as depth) into an image, laid over the picture by a pass after the clouds. If the
      engine's passes would also run for that camera, the writer stops and reports; nothing else is tried.
- **A7 Sounds (T5):** `sounds.json` holds, for each Wwise event the ship uses, what the exported banks say: its
  sounds and containers (random, sequence, switch, blend), loop flags, volumes, pitch, randomisation, the game
  parameter curves and states. `sound.rs` is a small mixer that plays that data (only the behaviours the ship's events
  use) from triggers proven in the exe or the notes (`manifest.json` `proven_triggers`, the state machines' cues at
  their times). A trigger or curve that is not proven is found in the exe or measured from the sessions' recordings,
  never tuned by ear; what cannot be found is listed, not faked. Sounds are `sfx/NAME.ogg`, the game's own packets
  rewrapped (grade A), each a `Need`.
  - **As built (2026-11-17/18):** the pure mixer is `se_core::elite_sound` (no Bevy, no device, no clock but its
    step's `dt`); the game side is `sidewinder/sound/`: `load.rs` (the data and the recordings, at the first spawn, on
    worker threads), `triggers.rs` (one function per entry of the study's `triggers.json`, named after it; nothing
    whose trigger is not proven is ever posted), `voices.rs` (everything that touches the audio device), `mod.rs`
    (the systems; plays only while seated; leaving by any path silences everything and resets the mixer).
  - **One output source (2026-11-18):** the mixer's heard voices are mixed by ONE source of our own on the audio
    thread (`voices.rs` `OutputSource`), not one audio entity a voice. Each voice is resampled there to 48 kHz
    (never by the audio library: its rate converter hangs on some rates), filtered by Elite's own low-pass and
    high-pass, then by the equalisers on its path; a frame's starts are prepared on worker threads and begin
    together at one sample; a virtual voice (not heard) is never built. Nothing on the audio thread may panic,
    block on the game thread, or allocate without bound. Where a sound sits (panning, distance) is this source's
    job too, by Wwise's own attenuation data, when the emitters' places are known; Bevy's spatial audio is not used.
  - **Effects:** `effects.json` (made beside `events.json` from the Init bank already exported, by
    `re/elite/tools/sidewinder_effects.py`; shipped inside `sounds.json`) lists every effect on every bus and node.
    A LINEAR effect whose maths is established from Elite's own copy of the plug-in (the Parametric EQ) is applied
    per voice, one stage for each node and bus on the voice's path to the master, with its curves and its bypass by
    states. An effect on a sound's own NODE is applied in that voice's chain whether it is linear or not (the Guitar
    Distortion, built in full from Elite's copy: exact per voice). An effect on a BUS that is not linear is applied
    per voice only where it is linear in the states the mod reaches (a distortion whose wet share is 0 there, with a
    test that says so); one that acts on the sum would need that bus mixed as a sub-mix in the output source, which
    is built only when such an effect is. The FIRST master limiter never limits at the ship's levels (the output
    counts the frames that would have), so only its output gain is applied. As built (2e02874): the master's last
    plug-in IS built and runs on the sum (see "The master chain" below; its limiter acts on a few hundred frames at a
    supercruise entry or a canopy breach); ONE bus (`aa299971`, the supercruise layers) is a real 12-channel sub-mix
    with Elite's compressor. Effects not built: the other buses' compressors (the gate test proves none would act),
    the reverbs, DTS's headphone plug-ins (off in the user's Elite settings); they are listed with their parameters in
    the notes.
  - **Routes, gains and the master chain (decided 2026-11-17, from the studies T5e, T5f, T5g, T5g-2; the build
    tables are their sections of `re/elite/notes/sidewinder_sound_events.md`).** The mixer stays "each voice straight
    to the stereo output": everything below is either a gain matrix of the voice or a stage on the output's sum.
    - **A voice's stereo gains are a matrix worked out by pure functions** in `se_core::elite_sound::place`, ported
      from Wwise's own code as the studies wrote it out (`re/elite/work/t5g/panmodel.py`, `panmodel3d.py`; their
      test vectors become unit tests). Four cases: no place or a place, on a stereo route or a 7.1.4 route. A bus
      mixes in its own channel layout if it has one, else its parent's (the exe's rule); four of the ship's buses are
      12-channel 7.1.4, and 332 of the 775 shipped sounds mix there. A 7.1.4 route is "pan to the 12 speakers, then
      Elite's own down-mix table to stereo": both linear, so they fold into the one matrix; the 12-channel mix itself
      is built for one bus only (below). The converter ships what the matrix needs (each bus's channel layout, each
      sound's centre share); the game never guesses a route.
    - **The gain stored with each recording** (`akd_gain_2`, already shipped and parsed) multiplies every voice whose
      media carry it.
    - **The compressors on the 12-channel buses** read the power averaged over the bus's channels, which only a real
      bus mix gives. They stay unbuilt, as identity, for as long as the counters prove none would act at the ship's
      levels (the 12-channel reading is about 7.8 dB lower than a stereo one, so the proof on stereo levels is on the
      safe side). If one would act, the writer stops and reports: that is the point where bus sub-mixes are built.
      **That point came for one bus (2026-11-18):** the supercruise layers' bus `aa299971` could not be proven
      either way, so it is a real sub-mix in the output source (`voices.rs` `BusMix`, `SUBMIX_BUS`): its voices go in
      with what they have before the bus's compressor (`VoiceUpdate::bus`: the gain up to the compressor, the 12
      channel gains, how many effects they run); the sum runs through Elite's own Wwise Compressor
      (`elite_sound::compressor`), is folded to stereo by Elite's table, and the effects after the compressor run on
      the folded sum (they must be linear and the same on every channel: the bank refuses otherwise). Built, the
      compressor turned out never to act at the ship's levels (5.6 dB under its threshold).
    - **The master chain** runs on the output source's sum, in Elite's order: the first limiter's output gain, then
      the last plug-in's four modules (a 6-band equaliser, a 4-band compressor, the channel gains, the soft limiter
      with its 14 dB of make-up), each ported from Elite's own copy of the code. Sounds of the game that are not the
      ship's never pass through it.
    - **The engine's inputs** (`triggers.rs`) are in Elite's units, read from Elite's audio update: percents of the
      full-power top speed, shares of the speed-limited turn rate, their derivatives over the physics time. The
      flight hands them over through `ShipFacts` (it owns the numbers; the sound never recomputes a flight rule).
- **A8 Working rules for this run (every writer and checker):**
  - **Ownership is per task:** a brief lists the files its writer owns; nobody edits a file outside it. Two writers
    never own the same file. A writer who needs a change in another's file reports it instead.
  - **Building:** iterate with `cargo build --profile agent` (exe `target/agent/spaceengine-rs.exe`, test runs with
    `PORT_BIN=target/agent/spaceengine-rs.exe`) and `cargo test --profile agent ...`; a release build of the game takes
    about 9 minutes, so build release once, at the end. If a build fails in a file you do not own, another writer is
    mid-edit: wait a minute and try again, never fix it. Save files in states that compile.
  - **Running the game:** only through `.audit/tools/runport.sh` (it holds the one-at-a-time lock). No game runs at
    all while a measuring session flies Elite (the brief says when).
  - **Commits** are made by the orchestrator after review, by path. Writers do not commit, push or change branches.
  - **Reports** carry the task's Build log entry (12 lines at most, the spec's style) and every number's source.
- **A9 The destination shown, and the mouse widget (2026-11-23, the user's first play: "it doesn't actually select the
  planet and i cant see where to go"; "there is no crosshair in the middle of the screen ... to show how much you are
  turning with the mouse just like in ED"):**
  - **What was wrong:** the Navigation tab's pick was stored (`Pilot::nav_body` / `Pilot::target`) and named in one text
    line on the target panel, but nothing in the view showed where it is; Elite's mouse widget was never built.
  - **One destination (U1, grade E: how Elite plays, not traced in the exe):** the ship has ONE locked destination, a
    body of this system (`nav_body`) or another system's star (`target`), never both. Every way of picking goes through
    one function on `Pilot` (the panel, T, `sw dest`, `sw target`): picking one clears the other; picking the locked one
    again unlocks it. J then does what Elite's one key does: a jump with a system locked, supercruise otherwise.
  - **`Pilot::dest` (a `DestMark`):** written once a frame by `fly` (it has the bodies and the ephemeris), read by the
    HUD and nothing else: the unit direction to the locked thing in SHIP space, its distance, its angular radius, its
    name, and whether it is a system. The HUD never looks a body up itself.
  - **What the HUD draws (all through `hud_anchor`, flat lines and text as the rest of the HUD: F where Elite's own art
    is not measured):** the locked row marked on the Navigation tab; a marker in the view at the destination's real
    direction (through `SeatView`, so it follows the head's turn) with its name and distance; the compass on a new anchor
    `COMPASS` at the decoded outline `cockpit_ui_hightlight_compass` (`!TutorialGuide_Compass`: place and size A): a ring
    and a dot, the dot filled with the destination ahead and hollow behind. The dot's law is a pure function
    (`compass_dot`): orthographic, grade E, because no recording shows Elite's compass or its marker in the view (nothing
    was locked in any of them: `re/elite/notes/sidewinder_hud_marks.md` C and R); for the same reason the compass is
    drawn only while something is locked (in the recordings, with nothing locked, there is none).
  - **The mouse widget (U2):** `sidewinder/mouse_widget.rs` holds the measured numbers and the pure shape function;
    `hud::draw_hud` calls it. It shows `Pilot::stick`, the mouse's virtual stick BEFORE the dead zone (the exe hands the
    raw value to a second output: `re/elite/ghidra/notes/0.11-flight.md`), unless the binds' `MouseGUI` is 0 (a missing
    setting counts as on: E), and never while a side panel is open (Elite doesn't draw it then: measured). Its look (the
    dot on the nose's axis, the split arrow that moves off it with the stick, the two cross-fading) is measured from
    session 1's and 2's videos (`re/elite/notes/sidewinder_hud_marks.md` W), in directions from the eye fixed to the
    ship, never drawn by feel. The law is the place of the arrow's PIXEL centroid; the triangle is drawn so that the
    drawn shape's own pixel centroid lands on it. A test's mouse is `sw mouse DX DY` (counts, as the measuring sessions'
    `keys.py`), which goes through the same line of `fly` as real counts.
  - **Names:** these two tasks are U1 and U2 (the user's first play); the spec's Build log already has older tasks
    named N1 to N4.
  - **The mouse's sensitivity (U3, 2026-11-23; the user, asked whether to use their Elite's 0.27 instead of Elite's
    default 1.0: "Yes, use 0.27"):** in Elite the mouse's sensitivity is the player's own setting: the preset file says
    1.0 and the player's own bindings file holds theirs (the user's `Custom.4.2.binds`: 0.2725, the value every
    measuring session was flown with). The mod ships Elite's preset files unchanged (`bindings_match_elite` checks
    them against the install), so the player's value lives in the port's own settings, never in the shipped files.
    - `sidewinder::MouseSetting` (a resource that is always there, mod on or off: `sensitivity: f32`), default
      `DEFAULT_MOUSE_SENSITIVITY` 0.2725 (the user's decision; Elite's preset 1.0 is 3.7 times as quick).
      `settings.rs` saves and loads it as `SidewinderMouseSensitivity` (a saved file without the key gets the
      default, so nobody's settings file needs touching; test runs skip the settings, so they fly at 0.2725 and
      `sw mouse 0 -100` is one step of the recordings). The Settings window has its slider (0.05 to 2: the range is
      ours, F; it must show and keep 0.2725 exactly until it is moved).
    - `fly` takes the counts' scale from that resource alone, through a pure function (`full_counts(sensitivity)`:
      `MOUSE_FULL_COUNTS` / sensitivity: 1,180 counts for full stick at 0.2725, 321.6 at 1.0); the binds'
      `MouseSensitivity` is no longer read. The dead zone and the linearity stay the binds' (the user's file has the
      preset's 0.05 and 1). The mouse widget is untouched: it shows the stick, whatever moved it.
    - Who reads whom: `settings.rs` and `settings_window.rs` read the mod's resource (as they read the music's and the
      sound's); the mod reads nothing of the settings.
- **A10 Elite's ship UI (2026-11-23; the user's task "Build Elite Dangerous's ship UI into the Sidewinder mod", then
  their scope: "i simply just want the UI/screens and sounds from the Elite Ship. dont make the scope to complex that
  adds any crazy features").** Everything decoded is in `re/elite/frea_export/sidewinder_ui/decoded/` (its `README.md`
  is the handoff; the notes are `re/elite/notes/ui_*.md`). Tasks are named UI0 to UI8 and US1 (the spec's Build log).
  - **The route (UI0, decided 2026-11-23 from a trial, `re/elite/work/ui/ruffle_spike/`): Elite's own movies are run,
    not rebuilt.** Each movie is a Flash file with its own ActionScript 3 code that lays everything out; the Ruffle
    Flash runtime (MIT/Apache, Rust, on wgpu 30 like the engine) ran the seven movies tried with no ActionScript
    error, drew the gauge sheet and the left panel as Elite's pictures show them, took typed messages and gave back
    sounds and events. Rebuilding each movie by hand would be months of work and never exact.
  - **Scope (the user's):** Elite's HUD, panels and screens, fed ONLY by what the mod already simulates, with Elite's
    UI sounds. No new game system is made to fill a page (other ships, missions, cargo, scanning gameplay): such a
    page or row shows empty, as Elite's own does with nothing to show. The galaxy and system maps are cut. Movies in:
    `cockpitgaugepanel`, `cockpithudring`, `targetsights`, `cockpitinfopanel`, `orbitalhud`, `notifications`,
    `cockpitexternalpanel` (left), `cockpitinternalpanel2` (right), `rolepanel3`, `cockpitchatpanel`, and last, plain,
    `fsshud` and `saahud`. Out: the fuel scoop gauge (the stock Sidewinder has no scoop), interdiction, wing, mining,
    services, landing pop-up, vignette, mouse cursor, text display, head-locked text, the start-up animation (unless a
    later task needs one of them to show something the mod has).
  - **Three stages, as for every Elite file:** the export (out of git) -> `copy_sidewinder` (`convert/ui_*.rs`, test
    only) -> `crates/se_engine/assets/mods/sidewinder/ui/` (each file a `Need`) -> the game, which reads only that.
  - **Modules** (each with one job; the arrows are the only dependencies allowed):

    | Module | Its one job | May depend on |
    |---|---|---|
    | `vendor/ruffle/` | Ruffle's crates at one pinned commit with our few patches (`UPSTREAM.md` lists the commit, each patch and why, and the `diff` command that re-checks it). Never edited otherwise; a newer Ruffle is taken whole | its own dependencies |
    | `crates/elite-ui` | Elite's UI RULES as pure code: the value type of messages (`Value`), Elite's text rules (the `$KEY:#field=value;` grammar, the number and unit formats, the string table), the focus state machine, and the feeders (from `UiFacts`, in Elite's units, to each movie's messages, change-driven as the exe's senders are) | std, serde. No Ruffle, no Bevy, no GPU, no files (bytes in) |
    | `crates/elite-gfx` | Running a movie: load a converted SWF, send it messages, step its clock, draw it into a GPU texture it is given, hand back what it sends (`Out`); its value-returning calls (Translate, GetString, LocaliseNumber...) go to a `Host` the game gives it | `elite-ui` (for `Value` only), `vendor/ruffle`, wgpu 30. No Bevy, no files |
    | `se_engine::sidewinder::ui` | The game's side: owns the movies, fills `UiFacts` from the mod, places and draws the quads, reads the keys, runs the focus machine, the scanner and the holograms | `elite-ui`, `elite-gfx`, the mod's public state (`Pilot`, `ShipFacts`, `Parts`, `Seat`, `ChairPose`), `sound::ui::UiSounds` |
    | converter | `convert/ui_movies.rs`, `ui_layout.rs`, `ui_models.rs` (the `gui_*` panel models and textures), `ui_sound.rs` (US1); `ui_hud3d.rs` when UI6 needs the scanner's and holograms' files | the export (test only), `swf` from `vendor/ruffle` |

    `ui` reads the mod and writes three things only: `UiSounds` (what to play), `UiActions` (what a panel asks the
    ship to do: `fly` does it, with the same rules as the keys, as `PanelAction` works today), and the seat's head aim
    and the "panel has the keys" flag (as `Panels` does today). The flight, `pilot`, `facts`, `parts` and the sound
    never read `ui`. The stand-in HUD and panels (`hud.rs`, `panels.rs`, `hud_anchor.rs`'s layouts) stay until the
    movies replace them part by part behind `SE_SW_UI` (1: Elite's UI; 0: the stand-ins; the default flips to 1 when
    the HUD and panels are in and checked, and the stand-ins are deleted in the wrap-up).
  - **Interfaces (fixed; a writer who needs one changed reports it):**
    - `elite_ui::Value`: `Null | Bool(bool) | Num(f64) | Str(String) | Array(Vec<Value>) | Object(Vec<(String, Value)>)`
      (an object keeps its fields in the order given).
    - `elite_gfx::Gpu::new(instance, adapter, device, queue)` (clones of the engine's own: one set of Ruffle's GPU
      objects shared by every movie). `elite_gfx::Movie` (not `Send`: it lives in a main-thread-only resource):
      `load(gpu, MovieFiles { name, swf bytes, fonts, host: Arc<dyn Host> }, target: wgpu::Texture) ->
      Result<Movie, String>`; `stage() -> (u32, u32)`; `ready()` (the movie called `SetReadyForInvokes`);
      `send(name, &[Value])` (the movie's `HandleMessage`; before `ready` it is queued in order); `store(context,
      property, Value)` (the DataStore); `step(dt_seconds)` (runs the movie's own frames at its own rate) -> whether
      anything may have changed; `draw()` (into the target, transparent where the movie is empty; only called when
      something changed); `drain() -> Vec<Out>`; `Out`: `Event { name, args }` (`SendEventToGame`), `Sound { name,
      source }` (`PlaySound`), `Action { name, args }` (`DoAction`), `Opened`, `Closed`, `Ready`, `Trace(String)`.
      `elite_gfx::Host` (`Send + Sync`) is the trait of the manager's calls that RETURN a value at once (`translate`,
      `get_string`, `localise_number` and the rest the movies call); the game implements it with `elite_ui::text`.
      As built (UI1, reviewed): `store` takes the context as a path (`&[&str]`) and cannot remove a node;
      `Out::Action` also carries the movie's `CapturingButtons`, `CapturingMouse` and `OnInputEvent` under their own
      names; `Host::get_database_item_data` has no default (the answer is the ship's); an object coming back from a
      movie has its fields sorted by name (Ruffle's order), so read fields by name. Two rules for whoever sends:
      Ruffle THROWS when a handler gets more arguments than it declares (send each handler's own count), and a
      handler stops at the first `undefined` it writes into a text field (an object must hold every field its handler
      reads on any path). Either comes out as an `Out::Trace`, never a panic.
      A movie's error is an `Err` or a logged line, never a panic in the game.
    - The converted movie: Elite's SWF with (1) the export tool's three slips mended (the file-attributes tag first,
      one image defined twice, an image on id 0, and the font-library import that names no symbols dropped: Ruffle
      would wait for that file for ever), (2) one code block of OUR ActionScript put before the movie's own
      (the first class of a name wins): the communication manager, the tweener, the DataStore and the tutorial
      manager that Elite's exe supplies natively, here forwarding to the host. Our ActionScript's source is
      `crates/elite-gfx/as3/` (in git) and its compiled block `crates/elite-gfx/as3/bridge.abc` is committed beside
      it (made by a script there with Ruffle's own compiler, which needs Java: a developer step, never part of a
      normal build; a test fails if the block is older than its source). The converter puts it in; the game never
      compiles or patches anything.
    - A normal `cargo build` needs no Java: `vendor/ruffle` carries Ruffle's built-in ActionScript already compiled
      (one of our patches lets the build use it); `UPSTREAM.md` says how to re-make it.
    - Shipped under `ui/`: `movies/<name>.swf`, `fonts/*.ttf` (registered under the font map's names `$TitleFont`
      ...), `strings/en.json` (the FULL English table the UI can ask for, not the subset), `layout.json` (from
      `rendering.json` and `panels_sidewinder_cockpit.json`: per movie its stage, bones, rig, quads or `gui_*` models
      with their uv rectangles, materials' values, background plates), `focus.json` (the `cockpitui` state machine)
      with `controls.json`, `models/gui_*.edm`, `hud3d/` (UI6), `sound_tables.json` (US1).
  - **Running the movies in the game (UI3):** each movie's target is a Bevy `Image` made once at its stage size (the
    notes' grade-B stand-in for the render-target size); the render world hands the image's `wgpu::Texture` back
    once and the movie draws into it with the engine's own device (no read-back, nothing swapped later: a material's
    bind group is not rebuilt when an image changes). A movie steps only while it is shown (a side panel only while
    open) and draws only when something changed. Budget: the seated HUD's movies together cost at most 2 ms of the
    main thread averaged per engine frame at 3440x1440 (measured with `SE_PERF`; a writer who cannot meet it reports).
  - **Where and how they are drawn (UI3):** by Elite's own data: the `gui_*` models (their uv rectangles cut the
    gauge sheet) and the four panel quads, on the cockpit model's bones or the chair rig's (so the panels sway with
    the chair, through `ChairPose`), placed by the same code that places the cockpit's nodes, never by maths of their
    own; the dark plates with their decoded alphas. The shaders are Elite's formulas from `rendering.json`
    (`GuiLighting` for the panels, `GuiBackground` for the plates, the uv-scale-offset one for the sights and the
    orbital HUD; the screen-space movies after the tone map), in `sidewinder_ui_*.wesl`, their per-frame values in
    one buffer written in place. What Elite's tone map does to those values is not decoded (the scene exposure input
    was never found), so the last step is ours: the shader's result is brought to the screen values measured on
    session ui1's pictures (grade C) and then passes back through the engine's own tone curve as the tunnel's shader
    does, so the HUD's colours on screen do not move with the scene's exposure.
  - **Data (UI4, UI5):** `elite_ui::UiFacts` is everything the UI shows, in Elite's units, filled once a frame by
    `sidewinder/ui/facts.rs` from the mod (never by a feeder reaching into the game). Each feeder is a pure function
    of it with the exe's own send rules (thresholds, floors, smoothing, formats) from `messages/*.json`, tested
    without the game. A field the mod has no source for is `None` and its message is not sent (the movie shows its
    empty state); nothing is invented to fill it. The Sidewinder's own values: `SetPanelsMode 0`, the speed scale
    100 / 160 / 220 m/s at 0 / 2 / 4 engine pips, the 500..8000 m scanner table, the altimeter record.
  - **Keys and focus (UI5):** the `cockpitui` state machine as data (`focus.json`), run by `elite_ui::focus` (pure):
    in go Elite's button names (from the mod's `.binds` and the fixed `ui` control set), out come the focused
    panel, its head-look bone and field-of-view channel, which movie receives `OnInputButton` / `ReceivingButtons`,
    and whether the flight keys are quieted. The head turn keeps the measured turns (`sidewinder_hud.md`: the
    camera graph's rule was not traced).
  - **The 3D parts (UI6):** the scanner and the two holograms are the game's own drawing in Elite too, so they are
    native here: meshes from `hud3d/radar_vertex_buffers.json`, the shaders ported from their disassembly, the
    palettes and models from `hud3d/`. Contacts are what the mod has (this system's bodies in supercruise; nothing in
    normal space), the target hologram's model by the body's class, the Orrery with nothing locked.
  - **Sounds (US1):** a movie's `PlaySound(name, source)` goes through its interface's own table to an event of the
    ship's one sound bank (`sound/ui.rs` `UiSounds::play`; no entry is silence), and the HUD sounds the exe posts
    itself go by name (`UiSounds::post`) from the same place that sends the matching message.
  - **Stated stand-ins (each named in its task's Build log entry):** the movies' render-target size is the stage size
    (B); the device brightness factor is 1.0 (E); the tweener's 41 curves are Penner's standard ones under Elite's
    own names and numbers (`core.tweening.Easing`; the exe's code was not read: B); the DataStore holds what the
    feeders put in and is empty otherwise; the tutorial manager says "no tutorial"; a sound's `source` does not pick
    an emitter (the listener's own); the head turn is the measured one; Scaleform's text extensions (no-translate,
    auto-size, vertical align, inline images) are implemented in the Ruffle patch from Scaleform's documented
    behaviour and checked against session ui1's pictures (C).
  - **Order:** UI0 the route (done). US1 the sounds. UI1 `vendor/ruffle` + `elite-gfx` + `elite-ui`'s `Value` and
    text rules, proven headless on every movie in scope. UI2 the converter (movies, fonts, strings, layout, models).
    UI3 the movies in the cockpit (placement, shaders, the look). UI4 the flight HUD's feeders. UI5 keys, focus and
    the side panels. UI6 the scanner and holograms. UI7 the FSS and surface scanner as plain screens. UI8 the checks
    against Elite's pictures, the stand-ins removed, release, notes. Two writers at once only on different files.

- **A10 addendum (2026-11-24): how Elite's UI ended up, where it differs from the plan above.**
  - *Its own place in the frame (UI3b).* `sidewinder/ui/pass.rs` (`ui_pass`, set `UiPassSet`) runs in `Core3d` `PostProcess` after
    the colour correction, FXAA and SMAA, before the upscaling, into each `ColorCorrection` view's main texture at the view's full
    size (never the solids' corner; nothing of the bloom, exposure or tone curve comes after). Values are display-referred: a GUI
    value goes through `screen_fit` and is written as it is. Geometry is one vertex and one index buffer made with the UI;
    per-piece records (model, ship scale, depth flag) are one buffer written in place; groups are made once per material and per
    view's depth texture. Depth follows Elite's flag per material, tested in the shader against the view's depth texture through
    `view.main_pass_viewport`, the fragment moved 1 % of its distance towards the eye (`DEPTH_BIAS`, only in
    `sidewinder_ui_common.wesl`). Order: plates, panels by render layer, then `scanner_pass`, then the holograms' `display_pass`
    (both `.after(UiPassSet)`, same views and target, importing `seat_z`, `depth_texel`, `ui_in_front`, `screen_fit`). The
    holograms' generation pass (a 512 buffer) stays its own. No Bevy `Material` draws any of the UI.
  - *On or off.* `ui::enabled()` reads `SE_SW_UI` once (on unless `0`). `ui::UiLive` is "made and running" and is what the
    stand-ins ask (`hud.rs`, `pilot.rs`); `ui/place.rs` `off` is the single way the UI turns off (`drop_all` resets `UiLive`,
    the FSS cover flag, `UiScreens`, `Panels`). The stand-in HUD and panels stay as the fallback; they were not removed.
  - *What the UI writes outside itself* (the plan said three things): `UiReplies` readers aside, it writes `Panels` (the carrier:
    `quiet`, `UiSets`, `fov_add_deg`, the actions `fly` performs), `Seat::screen` (the FSS covers: the head held, every ship
    action quiet through `pilot.rs` `ship_input`), `scanner::RadarKeys`, and `messages` only reads. Nothing in the flight reads `ui`.
  - *Movies' life.* Loaded one a frame while seated; stepped at their own rate, the heavy ones on separate frames; drawn only when
    `Movie::changed()`; dropped and loaded again on a new ship (`ui::new_ship`); stepped without drawing for `HIDE_LIMIT_S` after
    the pilot leaves the seat. Side-panel movies run only while the focus machine has them on screen.
  - *Screens.* `ui/screens.rs` `UiScreens`: the FSS as a plain screen (open rule, facts from the system's bodies, `covers()`).
    The surface scanner is not built (the stock Sidewinder carries none).
  - *Vendored runtime.* Six patches (a to f); f lets text reach its field's edge as Scaleform does.

- **A11 The parked ship is a sandbox prop (2026-11-24; the user after playing: "when i press Z to undo a prop, it does not get
  rid of the sidewinder", "i am not able to throw around or interact with the sidewinder using the physics gun").**
  - *What it is.* While nobody is seated and the sandbox's world holds the ship's place (the same conditions under which its
    static solid pieces were added until now: the world's frame on the ship's body, the ship within reach), the parked ship is
    ONE prop of the sandbox's world: `SandboxWorld::spawn` with `PropShape::Hulls` of the ship's own hulls, `Surface::Metal`,
    its mass the flight model's own. Everything a prop gets it then gets through its prop id, with NO special case in the
    weapons: the physics gun (grab, carry, throw, freeze), impacts and their sounds, collisions with props and the player, the
    props limit. Seated or flying, the ship is what it was before (the flight owns it; its static pieces as before): the prop is
    taken out on boarding and put back, at rest, at the ship's pose on getting out.
  - *One truth.* `Parked` stays the truth everything else reads (the draw, the shadow, the lights, the sound, boarding, the
    flight). While the ship is a prop, ONE system of the ship's module writes `Parked.pos` / `rot` from the prop's pose every
    frame (between the prop's last two ticks, as props are drawn); nothing else writes it then. A world that was remade (a new
    frame or body; a rebase is not one) gets the prop again at `Parked`.
  - *Undo.* The sandbox's undo list holds what was spawned in order, a prop or the ship (an entry of its own kind, not a prop
    id, so the prop coming and going with boarding does not lose the ship's place in the list). Z on the ship's entry, and
    "Clean up everything", ask the ship's module to remove the ship through the request the Mods row already uses
    (`ModControl::ship_request`); an entry whose ship is already gone is skipped, as a gone prop's is. The sandbox's code names
    nothing of `sidewinder::` beyond that request. Seated, Z is the ship's own key (flight assist): no undo.
  - *Not in scope.* Damage from being thrown; the Skate 3 mod's skater colliding with the ship (its world takes only static
    pieces: the ship as a prop is not one any more while parked).
  - *As built (2026-11-24), where it sharpens the above.* Boarding takes the pose as it is, and LANDED is decided by the
    flight's own touchdown test on that pose (`pilot.rs` `stands_landed`); a ship not standing on its gear is flying from
    there, at the prop's velocity, its thrusters from hover thrust. E never boards while a gun or the hands hold a prop. "The
    same ship" is `Parked::spawn` (a number raised once a successful spawn), which the machines, the chair and the sound read
    in place of comparing poses. The ship's prop never sleeps (rapier put a 45 t ship to sleep while it was still tipping
    onto its third foot); the spawn sets it down on the plane under its three feet. The cargo hatch's pieces are part of the
    prop's hulls.

## A12 Getting out anywhere (the user's 2026-11-24: "press B to disembark anywhere")

The user's decision, replacing the spec's D8 (Elite refuses to disembark unless landed; this is the user's choice, not
Elite's rule): B, and the role panel's Disembark, get the pilot out wherever the ship is. The user's words: "disembarking
in space makes the sidewinder despawn and since ... you cant be in garrys mod mode while in space ... you are in the
regular flying mode like when you press V". So the ship goes only where he cannot be put on foot.

- **Landed** (the flight's own `Pilot::landed`): as before. Out beside the ship, on foot; the ship stays, a prop again (A11).
- **Not landed, but within reach of the ground** (ground to stand on by the ship, `ground_by_ship`, and the LOWEST point of
  its model box, each corner against the ground under it, at most `BOARD_REACH_M` over that loaded ground, 6 m; the
  lowest point, not the origin, so a ship lying on its side on the ground counts as on it: the distance from which he can board it is the distance from which
  he can step out of it; no new number): the same as landed. Out beside it on foot (the point out from its left side in
  its level heading frame, `level_frame`, so an overturned ship does not put him under it), and the ship stays:
  `keep_in_world` sets it down on its feet where it is (`set_down_on_feet`) and it is a prop from then. The put-down
  WAITS for the ground: on the release frame the sandbox's ground field is still round where he last walked, so while
  `Parked::set_down` is pending the prop is not made until the field covers the feet (the ship is in the world as
  nothing meanwhile); made at the flown pose it dropped and slid after every real flight. This
  is the ship hovering low, or one boarded while tilted or lying on the ground (it flies from the moment it is boarded,
  A11): B must never delete a ship that is on the ground.
- **Anywhere else** (higher, over no ground he can stand on, the glide, supercruise, hyperspace's countdown and tunnel):
  the ship is gone (the way a lost ship goes: `take_out`, `Parked` None, its undo entry skipped) and he is in the port's
  free flight (`Mode::Fly`, what V gives) where the ship was. Nothing is set down and he is not sent to Earth. V from
  there walks if there is ground under the camera, as V always does.
- One rule, in `seat.rs`: `leave_rule(landed, by_ground, by_b)` gives `Out` (landed, or within reach of the ground,
  WHICHEVER way he leaves: a ship on the ground is never deleted by leaving it), `Abandon` (B / Disembark anywhere
  else) or `Lost` (the mod turned off any other way anywhere else: the ground under the ship or Earth, as before). Nothing refuses, so the role panel's Disembark is always enabled while
  seated (`leave_allowed`), in step with B.
- The camera (the 500 m-a-frame rule holds): at the release it is written once more exactly where the seat's camera had
  it the frame before, so its PLACE never moves at the release; its TURN is the ship's own without the head's, as
  before A12 (the fade from black covers it; with the head's turn he came out facing the panel he had looked at). Out of a
  ship that was not landed the turn is its level heading frame's (`out_turn`): from a tipped ship he would
  otherwise come out looking at the ground.
- As built (commit 75dceac, 2026-11-24): `seat.rs` (`leave_rule`, `Why`, `exit_rule`, `within_step_out`,
  `flight_by_ground`, `ground_under`, `seat_at`, `write_seat_camera`, `level_frame`, `out_turn`, `release`),
  `sidewinder/mod.rs` (`put_back`), `sandbox/mod.rs` (`Sandbox::free_flight`, `FLIGHT_START_SPEED`). Left open, older
  than A12: on sloping ground the ship's prop sometimes does not sleep and creeps (A11's sleep rule). Out of hyperspace that is the eye, bound to the ship's body as
  of this frame, at rest against it. While `pilot.jump` is set (the tunnel) it is the jump's pose in the universe,
  unbound and at rest, WHATEVER bodies still exist: the old system's bodies live on for the first frames of the tunnel
  (the loader keeps them while the target is generated), and binding to one of them threw the camera back to the old
  system. The tunnel's states end (`free_seat`); the free camera finds its own body.
- The sandbox's side is one method, `Sandbox::free_flight`, beside `set_down` (the mode, no pending set-down, the
  player's body parked), so the seat names no sandbox internals. The free flight's assigned speed is capped at V's pace
  (`FLIGHT_START_SPEED`) only when the ship was over ground it could stand on within `SOLID_REACH_M` (1,500 m: the reach
  within which the ship belongs to the sandbox's world, `flight_by_ground`); higher, in supercruise and in hyperspace it
  is left as it was.
- A leave request is acted on only once the sandbox has given the seat its mode (`Mode::Pilot`); on the boarding frames
  before that it is dropped (it would otherwise remove a ship he has not yet sat in).
- Destroyed and removed are unchanged. The mod turned off another way follows the same reach rule (above).
