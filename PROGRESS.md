# Progress

## Current state (2026-11-01)

- The port runs and covers most of SpaceEngine; see README.md "Status" and
  `docs/missing_features.md` for what is left.
- The work below is committed on the branch `nebulae-and-close-up-ground` (not yet merged into
  `main`, nothing pushed).

## Recently done

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
