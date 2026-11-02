# 03 Camera, navigation and input

## Paths in scope
- crates/se_engine/src/camera.rs (1.8k lines: free flight, orbit, follow/rotate-with, go-to, speed)
- crates/se_engine/src/universe.rs, crates/se_core/src/upos.rs, crates/se_core/src/coords.rs
- crates/se_engine/src/projection.rs (+ assets/shaders/projection.wesl)
- crates/se_engine/src/waypoints.rs, session.rs, switches.rs

## What it does
Keeps the camera in universe coordinates (big-precision position `UPos`), turns mouse/keyboard into
motion, handles Go to (G), follow / rotate-with-object, speed scaling by distance, waypoints/bookmarks,
the session (saving/restoring the camera between runs) and alternative projections (fisheye etc).

## Likely bug classes
- Input leaking: keys/mouse acting on the camera while typing in a text box or while the cursor is
  over an egui window (the info panel click-through was one such bug).
- Precision: converting UPos to f32/f64 too early, losing position far from origin; drift when
  switching reference object; jumps when the followed object changes.
- Go to / follow: target despawned mid-flight, target at zero distance, NaN from normalising a zero
  vector, go to an object that doesn't exist, cancel/restart.
- Session save/restore: partial writes, bad parse → panic or lost bookmarks, version mismatch.
- Frame-time dependence: speed or rotation that depends on frame rate, huge dt after a hitch.
- System ordering: camera transform computed before/after things that read it in the same frame.

## Build / run / test
See .bugfinder/HUNTER_BRIEF.md. Port runs can script `goto`, `select`, `fly`.
