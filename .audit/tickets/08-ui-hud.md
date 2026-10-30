# Ticket 08: User interface, HUD, maps and overlays

Read `.audit/README.md` first: it has the places, tools, rules and the exact output format.
Your ticket id is `08`; write `.audit/gaps/08.txt`; scratch files go in `.audit/work/08/`.

## Scope
Every SE window, panel, menu and on-screen overlay (from `eng-gui.cfg` sections and the exe): main menu
(`textures/common/Menu.pak`, splash screens, `banner.cfg`), toolbars (`Toolbars` 615-625), info/selection
panel and its fields, wiki, search/find, star browser, solar system browser (`SolarSystemBrowser` 625-636),
solar system chart (`SolarSystemChart` 636-651), universe map / star map / galaxy map modes
(`UniverseMap` 651-664, `star_map_*.glsl`, `galaxy_map_*.glsl`, `map_sphere.glsl`), labels, markers
(`body_mark.glsl`), orbits (`orbit*.glsl`), grids, constellations/asterisms if any, velocity/direction
indicators, object filters and overlay filters (`ObjectFilters` 664-688, `OverlayFilters` 688-754), debug
overlay (`DebugOverlay` 754-793), fps meter, `Interface` 525-578 and `Units` 578-615 keys, skins
(`Skins.pak`, `gui.glsl`), fonts, localization (27 languages), tooltips/hints, places window, console,
screenshot/notification messages.

## Notes and boundaries
Out of this ticket: graphics/settings window contents (09), camera/time controls behaviour (07),
spacecraft HUD (10). Report missing windows and missing fields/options inside existing ones.

## Port starting points
`crates/se_engine/src/{hud,overlay,browser,wiki,i18n,settings,star_search}.rs`, `crates/se_core/src/{locale,places}.rs`, docs `formats/config_locale.md`

## Method
1. List SE's features in this area from its own files: the shaders above (read them), the config keys of the
   named sections, `eng-gui.cfg` strings, `re/strings_*.txt` (grep for key names and messages), the catalog
   keys, and decompiled functions where behaviour is unclear.
2. For each feature, check the port (code, shaders, README Status, docs). Run the port when unsure.
3. Write only the gaps, in the README's line format, most visible first.
