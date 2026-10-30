# SpaceEngine `.se` scripts

Sources: the 12 shipped scripts in `data/scripts/Scripts.pak` (`CheckLoc.se`,
`menu/Scene1.se`, `menu/Scene1VR.se`, `tutorials/DefaultSettings.se`,
`tutorials/tut-00-interface.se` .. `tut-07-star-browser.se`, about 1000 commands in all),
the command table and keyword strings in `SpaceEngine.exe` (`re/strings_ascii.txt` plus the
table itself, read directly from the exe: each entry is a NUL-terminated lower-case name
followed by a handler pointer), and the port's own parser (`se_core::se_script`). "Observed"
means seen in a shipped script; "table" means the name is in the exe's command table but never
used by a shipped script, so its arguments are guessed from the engine's key strings.
Ghidra was not used; the handlers were not decompiled.

## Syntax

The language is the `.sc` catalog syntax (`se_core::sc`) used as a command list.

* One statement per line: `Command arg arg ...`. Command names are case-insensitive
  (the exe's table is lower case; scripts write `GotoLocation`, `WaitTrigger`, ...).
* Arguments are numbers (`3`, `2.0`, `-1e-5`), strings (`"Earth-Moon|Earth"`), bare words
  (`true`, `false`, variable names, `x`) and tuples in parentheses with spaces and/or commas
  (`(0 1 0)`, `(0.7, 0.2)`).
* A statement may be followed by a `{ ... }` block of `Key value` rows. The brace may open on
  the next line. One-line blocks are fine: `WaitTrigger "LoadingComplete" { MaxWaitTime 30 }`,
  `FOV { Time 0 }`.
* Comments: `// to end of line`, `/* ... */` (may span lines; `/**/` closes the comment-out
  idiom used in `Scene1.se`).
* There are no expressions, no variables in arguments, no labels. Statements run in order;
  `Wait*` commands block the script (the simulation keeps running).
* Hex fixed-point numbers appear in `GotoLocation` blocks: `Time` and each `Pos` component are
  signed hex integers scaled by 2^80 (`Time` is Julian Date, `Pos` is parsecs). They are the
  same encoding as `save/places*.cfg` (see `config_locale.md` section 6).
* Text strings use a tiny markup: `[BR]` is a line break, `[BIG]...[/BIG]` big text; other
  bracketed words (`[G]`, `[Shift]`, `[Space bar]`, `[Ok]`, `[DEG]`) are key or button labels
  left in the text.
* Object names are `Parent|Child` paths (`"Earth-Moon|Earth"`, `"Sun|Saturn"`,
  `"TRAPPIST-1|TRAPPIST-1 g"`); a plain name works as well.
* Paths: `Run "tutorials/DefaultSettings.se"` is relative to `data/scripts/`;
  `PrintTranslations "data/scripts/..."` is relative to the install root.

## Control flow and variables

| Command | Form | Notes |
|---|---|---|
| `If` | `if { Var < 7 }` ... `endif` | Observed: `if { LessonPassed < N }` around `SetForce LessonPassed N`. The block holds one row `Name op value` with `<`, `>`, `<=`, `>=`, `=`/`==`, `!=`, or just `Name` for non-zero. |
| `Elif`, `Else` | `elif { ... }`, `else` | Table only. |
| `Loop`, `EndLoop`, `Break` | | Table only (`break`, `loop`, `endloop`; argument form unknown, the port accepts `loop N`). |
| `Set` | `Set Name value` | 274 uses (settings, e.g. `Set ShowOrbits false`). Values are numbers or `true`/`false`. |
| `SetU`, `SetForce` | same as `Set` | `SetForce LessonPassed 3` writes a persistent progress counter. `SetU` is table only. |
| `Get`, `Reset` | | Table only. |
| `WaitVar` | `WaitVar Name value` or `WaitVar Name { Range (a, b) }` | Blocks until the variable equals the value or is inside the range (the user changes it through the GUI). |
| `SaveVars`, `RestoreVars` | | Tutorials bracket their changes: `SaveVars`, `Run "tutorials/DefaultSettings.se"`, ..., `RestoreVars`. |
| `Interpolate` | `Interpolate Name { To v Time s }` | Observed for `TimeScale` and `WaypointsOpacity`. |
| `Run` | `Run "path.se"` | Executes another script, then continues. |
| `CheckVersion` | `CheckVersion 991` | Aborts on older engines. |
| `Break`, `Restart`, `Quit`/`Exit`/`GTFO` | | Table only. |

Settings seen with `Set` (all booleans or numbers): `Show*` (`ShowOrbits`, `ShowConstellations`,
`ShowLabelsStars`, `ShowCatStars`, ...), `Enable*` (`EnableBloom`, ...), `LabelsMode`,
`OrbitMarksMode`, `PhotoMode`, `Exposure`/`ExposureComp`, `AmbientLighting`, magnitude limits
(`StarMagnLimit`, `GalaxyMagnLimit`, ...), point-sprite tuning (`PointScale`, ...), `Unit*`,
`LandLOD`, `Map*` and `Chart*` options, `SilentVarChange`. The full list is in
`tutorials/DefaultSettings.se`.

## Camera and selection

| Command | Form | Notes |
|---|---|---|
| `Select` | `Select "Earth-Moon|Earth"` | Selects an object; also selects waypoints by name. `Unselect` (and `Deselect`) clears. |
| `GotoLocation` | block | Jump to a saved camera location (below). |
| `Goto` | block | Fly to the selected object (below). |
| `Center` | `Center`, `Center true` (inside `Goto`) | Turn to face the selection. |
| `Follow`, `SyncRot`, `Free` | | Bind to the selection (translation only / also rotation), or release. `Track`, `Untrack` (table) keep the selection centred. |
| `Orbit` | `Orbit { Axis (0 1 0) AngularSpeed 10 }` | Orbits the selection about an axis at a speed in deg/s; `StopOrbit` ends it. |
| `FOV` | `FOV 45`, `FOV { Time 0 }` | Field of view in degrees; the block form with `Time` only resets it. `Time` is the transition time. |
| `SpeedKm` | `SpeedKm 500` | Sets the flight speed in km/s (`Speed` in table, in pc/s by the key names). |
| `UserMoveControl`, `UserRotationControl`, `UserTimeControl` | `"Disabled"`/`"Free"` | Lock or unlock the user's input. Also `UserStereobaseControl` (table). |
| `Fly`, `Turn`, `StopFly`, `StopTurn`, `StopRotation`, `StopMotion`, `Horizon`, `MoveMode`, `GotoURL` | | Table only. |
| `Waypoint` | `Waypoint "WP 1" { Parent "..." RadiusKm 500 Color (r,g,b,a) Shape "Circle" FixedPosPolarKm (lon lat dist) }` | Defines a marker in space (Shape: CIRCLE, TRIANGLE, SQUARE, DIAMOND, HEXAGON, SPHERE, WIREFRAME, BILLBOARD). `ClearWaypoints`, `DeleteWaypoint "name"`. Positions can also be `FixedPosXYZKm`, `StaticPosXYZKm`, `StaticPosPolarKm`. |
| `SplinePath`, `PlaySplinePath`, `DeleteSplinePath`, `ClearSplinePaths` | | Table only (camera paths). |

### `GotoLocation` block

Identical to a saved Place (`save/places-def.cfg`):

```
GotoLocation
{
    Body       "Menu Moon 1.2"
    Parent     "Menu Planet 1"
    Date       "2019.05.31 18:23:57.57"
    Time       +258367359E6C1E7E84FB2E0000     // optional, JD * 2^80, overrides Date
    Pos        (-4F5791FF91DD5932E1680000 ...)  // 3 x hex, parsecs * 2^80 from the Sun's centre
    Rot        (w x y z)                        // camera quaternion, looks along -Z
    Vel        2.0513983e-011                   // pc/s
    Stereobase 1
    BindMode   1                                // 0 free, 1 follow Body, 2 also rotate with it
    PhotoMode  0
    Exposure   0
}
```

It sets the simulation date, the camera and the selection (`Body`).

### `Goto` block

All keys optional; the target is the current selection (or `Object`):

| Key | Meaning |
|---|---|
| `Height` / `HeightKm` | altitude above the surface, km (observed: `Height 64000`) |
| `DistKm`, `DistRad` | distance from the centre, km or body radii (strings) |
| `Lat`, `Lon` | degrees on the body (east positive) |
| `Yaw`, `Pitch`, `Roll` | final view in the local horizon frame, degrees (`Pitch -90` looks straight down) |
| `Time` | flight duration, seconds (`0` jumps) |
| `Center` | `true` ends looking at the body's centre |
| `AccelTime`, `DriftTime`, `DecelTime`, `AccelPower`, `DecelPower` | flight profile (strings only) |

## Time

| Command | Notes |
|---|---|
| `TimeScale 50` | simulated seconds per second (negative runs backwards) |
| `Date "YYYY.MM.DD hh:mm:ss.cc"`, `Time +hex` | set the simulation date (table, form guessed from `GotoLocation`) |
| `StopTime`, `StartTime` | pause / resume (table) |
| `Wait 3.0` | block for 3 real seconds |

## Waiting for the user: `WaitTrigger`

`WaitTrigger "Kind|Name" { params }` blocks until the event happens. Several can be combined:
`BeginMultiTrigger "AND"` (or `"OR"`) ... `EndMultiTrigger` (`EndMultitrigger` also seen).
Common parameters: `MaxWaitTime s` (give up), `Object "path"`, `Type "Planet"|"DwarfPlanet"|"Galaxy"|"Selection"`,
`Range (a, b)`, `RangeKm (a, b)`, `RangeRad (a, b)` (distance in body radii), `YawRange`,
`PitchRange`, `RollRange` (degrees), `Rect (x0, x1, y0, y1)` (screen fractions), `Mode "OR"`,
`Dialog "name"`. Strings in the exe also show `ActivationDelay`, `Value`, `Direction`, `CROSSED`/`EARLIER`/`LATER`/`INRANGE`/`OUTRANGE`.

Kinds, with observed counts:

* Scene: `LoadingComplete` (10), `ScreenshotComplete`, `Time` (5; `Mode "Later"`, `Time "date"`), `EnterMenu`, `EnterPlanetarium`, `EnterSinglePlayer` (table).
* Objects: `Object|Approach` (48; distance to `Object` or a `Type` within `RangeRad`/`RangeKm`), `Object|Select` (22), `Object|See` (6; object inside `Rect`).
* Input (`Control|...`, 60+): `Goto`, `GotoCenter`, `Center`, `Land`, `Follow`, `SyncRot`, `Move` (`RangeKm`), `Rotate`, `Orbit` (`YawRange`/`PitchRange`), `ChangeSpeed` (`RangeKm` or `Range` in pc/s), `ChangeFOV`, `SetTimeRate` (`Range`), `PauseTime`, `ResumeTime`, `ReverseTime`, `SetTime`, `SetCurrentTime`, `MapZoom`, `MapFocus`, `MapCenter`, `ChartRotate`, `ChartCenter`, `StopMove`, ... Also `Input|Button`, `Input|JoyButton`, `Input|Custom`.
* Interface (`GUI|...`, 50+): `Open`, `Close`, `LeftClick`, `RightClick`, `SelectElement`, `SelectObject`, `ChangeValue`, `LockPanel`, `HalfLockPanel`, `UnlockPanel`, `ActionEnter/Back/Start/Stop/Next/Prev`.

## Messages and interface

| Command | Notes |
|---|---|
| `ShowMessage "text"` | show text in the tutorial message box and continue |
| `WaitMessage "text"` | show it and wait for the user's Ok |
| `HideMessage` | |
| `Print "text"`, `HidePrint`, `Log "text"` | on-screen text (table) and a log line |
| `HighlightWidget "Main toolbar|Wiki" { Time 2 Color (r,g,b,a) }`, `SetWidgetStyle "Message box" { Pos (0.7, 0.2) }` | tutorial hints for GUI widgets (169 and 14 uses) |
| `ShowDialog "Settings" { Tab "Graphics" }`, `HideDialog`, `HideAllDialogs`, `HideAllToolbars`, `OpenMenu "Main|Planetarium|Tutorial"`, `CloseMenu` | GUI windows |
| `StarBrowserReset { SearchRadius 10 MatchOnly false }` | star browser |
| `PlayMusic "file.ogg"`, `PauseMusic`, `ResumeMusic`, `FadeOut`, `FadeIn` | sound |
| `Screenshot`, `SavePNG`, `SaveTGA`, `SaveJPG`, `SaveDDS`, `SaveTIF` | table |
| `SetLocale "eng"`, `PrintTranslations "path"`, `ExportLocale`, `ImportLocale` | localization (used by `CheckLoc.se`) |
| `EnterPlanetarium`, `EnterSinglePlayer`, `ContinueSession`, `ShowFlightSchool`, `OpenStoreOverlay` | menu navigation (table) |
| `ShowObject`, `HideObject`, `Show`, `Hide`, `Bind`, `Scene`, `Map`, `Edit`, `Debug*`, `Rebuild`, `Print*`, `Watermark`, ... | other table entries |

The exe also reports script errors as `file, line N: ...` (for example "Camera movement is
limited, ignoring Goto command" when `UserMoveControl` is disabled) and has an
`ExitToMenuOnScriptError` option.

## Command table (from `SpaceEngine.exe`)

run, break, quit, exit, gtfo, restart, checkversion, screenshot, select, unselect, deselect, goto,
center, horizon, follow, syncrot, free, track, untrack, gotolocation, gotourl, fov, movemode,
speed, speedkm, orbit, stoporbit, fly, stopfly, turn, stopturn, stoprotation, stopmotion,
stoptime, starttime, timescale, time, date, wait, waittrigger, beginmultitrigger,
endmultitrigger, waypoint, deletewaypoint, clearwaypoints, clear, splinepath,
deletesplinepath, clearsplinepaths, playsplinepath, print, hideprint, log, hidealldialogs,
hidealltoolbars, showdialog, hidedialog, openmenu, closemenu, enterplanetarium,
entersingleplayer, continuesession, showflightschool, openstoreoverlay, starbrowserreset,
showmessage, waitmessage, hidemessage, setwidgetstyle, highlightwidget, playmusic,
pausemusic, resumemusic, showobject, hideobject, show, hide, set, setu, setforce, reset,
interpolate, get, waitvar, savevars, restorevars, resetvrpose, vrcontrollermode,
vrcontrollerdefault, scene, map, debuggen, rebuild, edit, savepng, savetga, savejpg, savedds,
savetif, optimizesm, watermark, printvars, printnames, printfont, exportfont,
printtexcachelog, printtranslations, dbsort, setaudiodevice, fadeout, fadein, displaymode,
togglestereoscopic3d, exportvractionmanifest, usermovecontrol, userrotationcontrol,
usertimecontrol, userstereobasecontrol, reloadconstellations, updatevrcontrollertype,
setlocale, exportlocale, importlocale, bind, if, elif, else, endif, loop, endloop.

## How the port runs them

`se_core::se_script::parse` turns text into a flat `Vec<Command>` (typed `Op`s, anything else
kept as `Op::Other`) with `if`/`loop` jump targets resolved. `se_engine::scripting` executes it:

| Command | Port behaviour |
|---|---|
| `GotoLocation` | camera pose, date and selection; `BindMode` 1/2 follow the body. Very distant bodies (the menu system, ~22 kpc) are placed in the catalog from rounded RA/Dec/Dist (error ~2e-3 pc, about 5e13 m), so the saved camera ends up far from the body and its true offset is lost; the port then frames the body together with its parent from above the orbital plane (or the body alone from 3 radii). |
| `Goto` | smooth flight in the body's frame to `Height`/`DistKm`/`DistRad`, `Lat`/`Lon`, `Yaw`/`Pitch`/`Roll`, `Time`; ends bound to the body |
| `Select`, `Unselect`, `Center`, `Follow`, `SyncRot`, `Free`, `Orbit`, `StopOrbit`, `StopMotion`, `FOV` | camera/selection (`FlyCamera`) |
| `TimeScale`, `Date`, `Time`, `StopTime`, `StartTime`, `Interpolate TimeScale` | `SimTime` |
| `Wait`, `WaitTrigger`, `BeginMultiTrigger`/`EndMultiTrigger`, `WaitMessage`, `WaitVar` | blocking waits; supported triggers are `LoadingComplete` (1 s), `Time`, `Object|Select/Approach/See`, `Control|Goto/Move/Rotate/Orbit/ChangeSpeed/SetTimeRate/PauseTime/ResumeTime/ReverseTime/ChangeFOV/Center/Follow/Land`. Triggers with no counterpart (all `GUI|...`, map and chart) time out after 4 s. |
| `ShowMessage`, `WaitMessage`, `Print`, `Log` | egui caption (`WaitMessage` has an Ok button) |
| `Set`, `SetForce`, `WaitVar`, `SaveVars`, `RestoreVars`, `If`/`Elif`/`Else`/`Loop` | script variables; `ShowOrbits`, `LabelsMode`, `StarMagnLimit`, `TimeScale` and `FOV` also drive the port's settings |
| `Run`, `Quit`, `Restart` | nested scripts, exit |
| everything else (`HighlightWidget`, dialogs, `UserMoveControl`, waypoints, music, locale) | logged once and skipped |
