# 09 Spacecraft: flight physics, autopilot, warp, ship HUD, saved ships

## Paths in scope
- crates/se_engine/src/ship.rs (2.3k lines), ship/*.rs (aero glue, air, autopilot, engine_sound,
  envmap, hud, manager, persist, warp), ship_assets.rs
- crates/se_core/src/ship.rs, ship/aero.rs, ship/aero_table.rs

## What it does
SE's spacecraft mode: loading ship models and their parameters, Newtonian flight with engines and
RCS, atmospheric drag/lift, gravity, landing/collision with the ground, autopilot (approach, orbit,
land), warp drive, the ship HUD, and saving/loading ships (persist.rs).

## Likely bug classes
- Physics: frame-rate dependent integration, explosion on a huge dt after a hitch, NaN from zero
  mass/zero velocity normalise, gravity from the wrong body or in the wrong frame after the active
  system/parent changes, tunnelling through the ground at high speed, collision with terrain that
  isn't loaded yet.
- Autopilot: never reaching its goal (softlock), oscillating, a target that disappears.
- Warp: leaving warp inside a planet, warp speed in wrong units.
- Save/load (persist.rs): partial writes, bad parse → panic or lost ships, version mismatch, saving
  a position that can't be restored after the universe changes.
- Input: ship controls active while typing or while the cursor is over UI; controls fighting the
  free camera.
- Leaks: ship entities/meshes/sounds not cleaned up when leaving the ship or switching ships.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. se_core's ship code can be tested in a scratch crate. Check README.md
for how to enter the ship from the command line or dev commands.
