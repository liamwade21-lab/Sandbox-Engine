# Progress

## Current state (2026-11-01)

- The port runs and covers most of SpaceEngine; see README.md "Status" and
  `docs/missing_features.md` for what is left.
- The work below is committed on `main` (nothing pushed).

## Recently done

- **Go to rotates with planets (2026-11-01):** Go to (G, and double G) now binds the camera as
  "Rotate with object" for planets, moons and other bodies, so their surface holds still under
  the camera; stars, galaxies and the rest are still followed. SE itself only follows ("Following
  Moon" in its tutorial); this is the user's choice. Checked at 3600x time on Mars: the camera
  stays at the same point in Mars's frame. Seen while testing: at very high time rates the info
  panel's distance wobbles (it samples a moving planet at a slightly different moment); harmless.

- **Info panel counts (2026-11-01):** the top-left info panel's "N children" row is replaced by
  SE's counts, each opening to a list: a star or star system shows "Number of planets",
  "Number of dwarf planets" and "Number of stars"; a planet shows "Number of moons". Counts look
  through barycentres (the Sun has 8 planets including Earth; Earth has 1 moon, Pluto 5) and skip
  asteroids, comets and spacecraft. Clicking those rows no longer also selects the star behind
  them (the panel is on egui's background layer, which egui does not count as interface).

- **Ground stops refining from the second landing on (2026-11-01):** each planet keeps up to
  1500 generated height grids; past that, the cleanup threw away every newly arrived grid whose
  terrain patch didn't exist yet, though the patch can only be built from it. So after a first
  landing filled the cache, new terrain under the camera could no longer be built: it stayed
  coarse, or flickered between sharp and blurry. Now a grid is kept for 10 s after it was last
  used. Checked with three fast landings on RS 8514-1828-5-27938-309 2: all reach the finest
  terrain and stay sharp. This is probably also behind the user's original "limit to how close
  I can get" report.

- **Nebulae (2026-10-31):** procedural nebulae are volumetric like SE's, nearer nebulae draw
  over farther ones, and nebulae no longer pop in and out. The user confirmed the look.
- **Ground up close (2026-11-01)**, for the user's two reports ("a limit to how close I can get"
  and "the surface fades into a different texture as I get closer"):
  - Ground textures no longer blur into blobs when the camera is a few metres up: a finer copy of
    the same texture fades in as it gets magnified, so the ground stays sharp down to 1.5 m.
  - The ground keeps its colour at every distance (it used to turn from orange to gold, with
    green and red specks, on the way down).
  - Close-range texture now also shows on coarser terrain patches, so while finer terrain is
    still loading the ground at your feet no longer looks like it is hundreds of metres away.
  - The near clipping plane is now always set before the frame's camera is built (it could stay
    11 m out on Eris and cut away the ground in front of the camera).
  - The terrain generator only falls back to the port's own heights when it truly fails, not
    when it is slow (e.g. window minimized), which used to give planets holes and height jumps.
  - Checked on Eris, the purple planet RS 238-21-2-44-404 6 (Draco Dwarf) and Earth; all tests
    pass.

## Notes

- The user's SpaceEngine has a graphics mod installed, so SE screenshots don't show vanilla
  SE's exact textures.
- Seen while comparing: SE has Eris's spot at lat -36.29 lon -0.65 near nightfall around
  2026-10-30 19:00, while the port still has it well lit then. Eris's day/night timing may be
  off in the port (not looked into).
