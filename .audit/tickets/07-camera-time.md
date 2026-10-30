# Ticket 07: Camera, navigation, controls and time

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `07`; write `.audit/gaps/07.txt`; scratch files go in `.audit/work/07/`.

## Scope
Camera modes (free, follow, sync rotation / surface lock, orbit, track/look at, chase), Goto/Center/
Follow/Land behaviour and timing, automatic speed and acceleration/inertia, velocity presets
(main-def.cfg `Controls` 793-823, `VelocityPresets` 823-854, `Physics` 439-453), ground collision, every
keyboard/mouse/gamepad binding SE has (controller cfgs `config/def_generic_controller.cfg`,
`def_xbox360_controller.cfg` list SE's action names; keybinding strings in the exe and `eng-gui.cfg`),
mouse sensitivity/smoothing/inversion, FOV, roll, the time system (time rate steps, reverse, pause, real time,
the date/time dialog, light-time options), reference frames and coordinate systems, `Ephemerides` 854-863,
saved places/locations behaviour (go to place, save place), console commands for navigation.

## Notes and boundaries
Out of this ticket: spacecraft flight (10); camera projections and stereo (09); UI windows other than the
navigation/time controls themselves (08).

## Port starting points
`crates/se_engine/src/{camera,universe,hud,scripting}.rs`, `crates/se_core/src/{time,coords,upos,places,ephemeris,jpl_de,orbit,rotation}.rs`, README Controls

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
