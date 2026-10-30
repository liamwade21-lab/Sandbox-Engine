# Ticket 10: Scripts, audio and spacecraft

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `10`; write `.audit/gaps/10.txt`; scratch files go in `.audit/work/10/`.

## Scope
Scripts: every command and parameter of SE's `.se` script language (command table in the exe, all scripts
and tutorials in `data/scripts/Scripts.pak`), triggers, variables, tutorials' GUI hooks, the console.
Audio: `Sound` 424-439 keys, `data/sounds/menu`, `data/sounds/spacecraft` (engine, warp, UI sounds), music
context switching (`data/music/context.cfg`: which classes/situations pick which tracks, fades, volume).
Spacecraft: `models/spacecraft/ShipsModels.pak`, `textures/spacecraft`, `ship.glsl`, `ship_pbr.glsl`,
`ship_engines.glsl`, `ship_*_engine.glh` (engine plume types), warp/hyperdrive visuals (`WarpRendering`
184-196), flight model, autopilot, docking, landing gear, lights, HUD of the ship, aerodynamics, fuel,
damage, multiple ships, ship selection UI.

## Notes and boundaries
Out of this ticket: the ship editor (out of scope), free-camera navigation (07).

## Port starting points
`crates/se_engine/src/{scripting,music,ship,ship_assets}.rs`, `crates/se_core/src/{se_script,ship,model_script,dds}.rs`, shaders `ship.wesl`, `ship_plume.wesl`, docs `formats/se_script.md`, `formats/spacecraft.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
