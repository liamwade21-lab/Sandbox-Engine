# SpaceEngine `.se` scripts

Sources: the 12 shipped scripts in `data/scripts/Scripts.pak` (`CheckLoc.se`,
`menu/Scene1.se`, `menu/Scene1VR.se`, `tutorials/DefaultSettings.se`,
`tutorials/tut-00-interface.se` .. `tut-07-star-browser.se`, about 1000 commands in all),
the command table and keyword strings in `SpaceEngine.exe` (`re/strings_ascii.txt` plus the
table itself, read directly from the exe: each entry is a NUL-terminated lower-case name
followed by a handler pointer), and the port's own parser (`se_core::se_script`). "Observed"
means seen in a shipped script; "table" means the name is in the exe's command table but never
used by a shipped script, so its arguments are guessed from the engine's key strings.
The table (`.rdata` 0x14075f9c0..0x1407610a0) has 0x30-byte entries: a u32, the name (36
bytes) and the handler. Handlers decompiled for the port: `goto` FUN_140563f30, `center`
FUN_140563cf0, `fly` FUN_140562dd0, `turn` FUN_140562670, `stopturn` FUN_1405623a0, `stopfly`
FUN_140562b00, `speed` FUN_140561b00, `speedkm` FUN_140561a40, `movemode` FUN_140561bb0,
`checkversion` FUN_140565510, `waypoint` FUN_1405614a0 (FUN_140065f40, FUN_140549e90),
`deletewaypoint` FUN_140561280, `highlightwidget` FUN_14055df80, `setwidgetstyle`
FUN_14055e790, `hidealltoolbars` FUN_1402536d0, `playmusic` FUN_14055dd30, `pausemusic`
FUN_14055dc00, `resumemusic` FUN_14055dad0, `fadeout` FUN_1405554e0, `fadein` FUN_140554f90,
`usermovecontrol` FUN_140552c30, `userrotationcontrol` FUN_140552900, `usertimecontrol`
FUN_140552330, `userstereobasecontrol` FUN_140551ef0, `gotourl` FUN_140563720; triggers are
parsed by FUN_140276ca0 and fired by FUN_140276120 (`Control|Move`: FUN_140275dd0). The script
cleanup at the end of a script is FUN_14054d240 / FUN_140221b10.

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
| `SpeedKm`, `Speed` | `SpeedKm 500`, `Speed 0.5` | Sets the assigned velocity in km/s, or pc/s. |
| `UserMoveControl`, `UserRotationControl`, `UserTimeControl`, `UserStereobaseControl` | `"Disabled"`/`"Free"`/`"Limited" { ... }` | Lock or unlock the user's input. Move "Limited" keeps the camera within `DistKm`/`DistLy`/`DistRad` of `Center` at up to `SpeedKm`/`Speed`; time "Limited" keeps the date in `TimeRange` and the rate in `TimeRateRange`; stereobase "Limited" takes `Range`. While movement is disabled or limited (both raise SE's flag) the user's Go to, Land and binding are refused, and a script's `Goto` is refused ("Camera movement is limited, ignoring Goto command") and so are `Follow`/`SyncRot`/`Free`; while rotation is disabled `Center` is. |
| `Fly` | `Fly { Axis (0 0 1) Speed 10 SpeedKm 100 FadeTime 2 Func "linear" }` | Flies along a camera axis (x left, y down, z ahead) at pc/s or km/s, reached over `FadeTime`; defaults 10 pc/s, 2 s. `StopFly { FadeTime }` slows to a stop. |
| `Turn` | `Turn { Axis (0 1 0) AngularSpeed 10 FadeTime 2 }` | Turns about a camera axis in deg/s; `StopTurn { FadeTime }`; `StopRotation` stops at once. |
| `StopMotion`, `Horizon`, `MoveMode 1..4`, `GotoURL "se://..."`, `Bind` | | `Bind` binds VR controller buttons. |
| `Waypoint` | `Waypoint "WP 1" { Parent "..." RadiusKm 500 Color (r,g,b,a) Shape "Circle" FixedPosPolarKm (lon lat dist) }` | Defines (or, by name, changes) a marker in space. Keys (FUN_140549e90): `Parent`, `RadiusKm`/`Radius` (pc), `Roll`, `FadeRangeKm`/`FadeRange`, `LabelFadeRangeKm`/`LabelFadeRange`, `Color`, `ColorEmpty`, `LabelColor`, `LineWidth`, `Shape` (CIRCLE, TRIANGLE, SQUARE, DIAMOND, HEXAGON, BILLBOARD with `Texture`/`TexCoord`/`TexCoordPix`, SPHERE, WIRESPHERE), the position `StaticPosXYZ[Km]`, `StaticPosPolar[Km]`, `FixedPosXYZ[Km]`, `FixedPosPolar[Km]` (Fixed turns with the parent), and `Visible`, `Label`, `Triggerable`. New waypoints: radius 1 km, white, line width 2, a circle. `ClearWaypoints`, `DeleteWaypoint "name"`; the list is cleared when a script ends. |
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
| `AccelTime`, `DriftTime`, `DecelTime`, `AccelPower`, `DecelPower` | seconds speeding up, drifting and braking (defaults 1, 0, 1; `Time T` makes speeding up and braking T/2 each, FUN_140571db0) and the curves' powers (default the `GotoSpeed` setting, 10); the flight lasts the three times' sum |

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
`Dialog "name"`, `Direction (x y z)` (`Control|Move`, camera axes x left, y down, z ahead; the
movement must match it within 1% per axis), `LonLatRange (lon0 lon1 lat0 lat1)`
(`Object|Approach`), `TimeRange ("date", "date")` with `Mode "InRange"`/`"OutRange"` (`Time`;
`Mode` also `"Crossed"`, `"Earlier"`, `"Later"` against `Time`), `ActivationDelay`, `Value`
(`GUI|ChangeValue`), `AnimateObject`, `Button`. `Control|Move` sums the user's movement since
the wait began and fires while its length is inside `RangeKm` (or `Range`, parsecs).
`tut-02-free-mov.se` writes `Direction (0.0, 0.0, s-1.0)`; the port reads `-1.0`.

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
| `HighlightWidget "Main toolbar|Wiki" { Time 2 Freq 4 Color (r,g,b,a) }` | the widget blinks in `Color` (default (1, 0.2, 0.2, 1)) `Freq` times a second (default 4) for `Time` s, or with no `Time` until the next trigger or `WaitVar` ends; `""` clears it. Special names: `"Message box"`, `"SelectionPointer"` (169 uses) |
| `SetWidgetStyle "Message box" { Pos (0.7, 0.2) }` | a widget's top-left corner at a fraction of the screen (`PosPix`: pixels); also `WinColor`, `TextColor`, `Visible`, `Highlighted`, `Active`, `Disabled` (14 uses) |
| `ShowDialog "Settings" { Tab "Graphics" }`, `HideDialog`, `HideAllDialogs`, `HideAllToolbars`, `OpenMenu "Main|Planetarium|Tutorial"`, `CloseMenu` | GUI windows; the four toolbars are dialogs too: `HideAllToolbars` puts them in auto-hide (they slide back under the mouse) |
| `StarBrowserReset { SearchRadius 10 MatchOnly false }` | star browser |
| `PlayMusic "file.ogg" { CrossfadeDuration 1 }`, `PauseMusic`, `ResumeMusic` | music; the cross-fade defaults to `MixDurationScript` (1 s) |
| `FadeOut { Time Color DefTime }`, `FadeIn { Time }` | fade the screen to a colour (default black) and back; `DefTime` names a default time (`overlayfadeout`, `guislidein`, ...) |
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
| `GotoLocation` | camera pose, date and selection; `BindMode` 1/2 follow the body. Very distant bodies (the menu system, ~22 kpc) are placed in the catalog from rounded RA/Dec/Dist (error ~2e-3 pc, about 5e13 m), so the saved camera ends up far from the body and its true offset is lost; the port then frames the body together with its parent from above the orbital plane (or the body alone from 3 radii). `PhotoMode`, `Exposure` and `Stereobase` have no counterpart. |
| `Goto` | flight in the body's frame to `Height`/`DistKm`/`DistRad`, `Lat`/`Lon`, `Yaw`/`Pitch`/`Roll`; ends bound to the body. Timing as SE (FUN_14056b1e0): a bare Goto takes 2 s, `Time` and the profile keys as above; each phase covers its share of the progress, speeding up as (t / accel)^AccelPower, braking as 1 - (1 - u)^DecelPower. Refused while movement is locked or limited; `Center` ignored while rotation is locked. |
| `Select`, `Unselect`, `Center`, `Follow`, `SyncRot`, `Free`, `Track`, `Untrack`, `Horizon`, `Orbit`, `StopOrbit`, `Fly`, `StopFly`, `Turn`, `StopTurn`, `StopRotation`, `StopMotion`, `Speed`, `SpeedKm`, `FOV` | camera/selection (`FlyCamera`); `Select` takes waypoints too (`waypoints.rs`) |
| `UserMoveControl`, `UserRotationControl`, `UserTimeControl` | `camera::InputLocks` (movement keys, mouse orbit/zoom/look, Go to, Land, Center, Horizon, Track, binding) and a time guard (the time keys are swallowed, other time changes undone; "Limited" clamps). Cleared when the script ends. `UserStereobaseControl` is accepted (no stereo view). |
| `TimeScale`, `Date`, `Time`, `StopTime`, `StartTime`, `Interpolate TimeScale` | `SimTime` |
| `Wait`, `WaitTrigger`, `BeginMultiTrigger`/`EndMultiTrigger`, `WaitMessage`, `WaitVar` | blocking waits; supported triggers: `LoadingComplete` (1 s), `Time` (all modes), `Object|Select/Approach/See` (objects and waypoints, `Type`, `LonLatRange`), `Control|Goto/GotoCenter/Move` (summed movement, `Direction`) `/Rotate/Orbit` (yaw, pitch, roll) `/ChangeSpeed/SetTimeRate/PauseTime/ResumeTime/ReverseTime/SetTime/SetCurrentTime/ChangeFOV/ResetFOV/Center/Horizon/Follow/SyncRot/Free/Track/Untrack/Land/StopMove/StopRotate`, `GUI|Open/Close/SelectObject` on the port's windows (Wiki, Settings, Find object, Locations, Solar system browser, Star browser) and `GUI|LeftClick` on the navigation toolbar's date. `ActivationDelay` holds a trigger back. The rest (universe map and chart controls, panel pins, clicks and values inside dialogs, `Input|*`) time out after 4 s. |
| `ShowMessage`, `WaitMessage`, `Print`, `Log` | egui caption (`WaitMessage` has an Ok button), placed by `SetWidgetStyle "Message box"` |
| `HighlightWidget` | a blinking frame over the HUD button (buttons record their rectangles under SE's names), its toolbar or window when the port has no such button, the caption, or the selection (`script_gui.rs`) |
| `ShowDialog`, `HideDialog`, `HideAllDialogs`, `HideAllToolbars`, `OpenMenu`, `FadeOut`, `FadeIn` | the port's windows and toolbars (sliding out and back in under the mouse), the Scripts window for SE's tutorial menu, a screen fade |
| `PlayMusic`, `PauseMusic`, `ResumeMusic` | `music::ScriptMusic` |
| `Waypoint`, `DeleteWaypoint`, `ClearWaypoints` | `waypoints.rs`: shapes drawn at their size in the view, hidden behind their parent, with labels and a selection pointer |
| `Set`, `SetU`, `SetForce`, `Get`, `Reset`, `Interpolate`, `WaitVar`, `SaveVars`, `RestoreVars`, `If`/`Elif`/`Else`/`Loop` | `script_vars.rs` maps about 55 settings (star points and deep-sky magnitude limits, overlays and their per-kind filters, constellations and grids, bloom, lens flares, `LandLOD`, catalog and procedural clusters, procedural galaxies and nebulae, `ExposureComp`, `Brightness`/`Contrast`/`Saturation`/`Gamma`, `WaypointsOpacity`, `TimeScale`, `FOV`, `AutoHorizon`, ...); others stay script variables. `Reset` puts a setting back to the port's default (SE: the registered default, FUN_14054f320). `LessonPassed` is saved to the port's user folder, except in test runs. A script stopped by hand gets its first `SaveVars` back; a menu scene (SE runs it in the main menu's own variable slot, `EnableVariableSlots`) gives the user's settings back and shows the interface again when it ends. Music a script paused plays again when it ends. |
| `CheckVersion` | another version: a message, then the script stops |
| `Run`, `Quit`, `Restart` | nested scripts, exit |
| everything else (`GotoURL`, `MoveMode`, `Bind`, `StarBrowserReset`, `SetAudioDevice`, locale tools) | logged once and skipped |

Testing: `--script PATH --script-auto` lets waits for the user end on their own (after 8 s, or
`SE_SCRIPT_AUTO_S`) and logs every wait as met or ended; the dev commands `vel X Y Z` (a push of
the free-flight velocity) and `waitwp N` (hold until a script has N waypoints) help drive them.
