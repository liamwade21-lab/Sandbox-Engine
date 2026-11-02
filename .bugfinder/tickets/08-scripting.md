# 08 Scripting, script GUI, audio and messages

## Paths in scope
- crates/se_core/src/se_script.rs (SE's .se script parser), model_script.rs
- crates/se_engine/src/scripting.rs (2.2k lines: runs SE scripts and the port's --run dev commands),
  script_vars.rs, script_gui.rs
- crates/se_engine/src/messages.rs, music.rs, sound.rs, wiki.rs

## What it does
Parses and runs SpaceEngine's .se scripts (tours, tutorials: Goto, Wait, Select, Set... commands),
plus the port's own dev command language (`--run "select X; goto Y; shot Z"`), script variables,
script-driven GUI, the music player, sound effects, on-screen messages and the wiki pages.

## Likely bug classes
- Softlocks/hangs: a Wait that never finishes (waiting on a goto that was cancelled, or on an object
  that doesn't exist), loops, a script that blocks input forever, a script never cleaned up.
- Panics on malformed scripts: unterminated strings/blocks, missing args, number parse, unknown
  commands.
- Wrong results: arguments in wrong units, commands applied to the wrong object, variables not
  scoped, time commands mis-parsing dates.
- Audio: music player stuck, files never closed, sounds not stopped when a script ends, decoding
  errors crashing.
- Input leaking while a script GUI text box is focused.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. Scratch crates can parse every .se script in the install (see
crates/se_core/examples/dump_scripts.rs, script_survey.rs). Port runs can use `--run`.
