# The Sidewinder's cockpit, recreated (grade E) from the user's screenshots

Ticket B05 (spec step 0.5, cockpit). Elite's own cockpit model is in its encrypted files and was never opened; nothing here
comes from Elite's data. The only references were the user's screenshots `re/elite/screenshots/2026-11-14_user/1-9.webp`.

## Files

- `cockpit.obj` / `cockpit.mtl`: 3,212 triangles, flat normals, box-projected uvs (1 uv unit a metre). Objects and materials
  `frame`, `dash`, `console`, `seat`, `floor`, `glass`. Space: metres, X right, Y up, -Z forward, origin at the pilot's eye.
  Bounds x -0.78..0.78, y -1.15..0.95, z -2.13..0.45. The glass is its own object, one-sided, every triangle facing the eye
  (checked: 97 of 97). Closed solids (beams, cylinders, seat boxes) face outward; one-sided surfaces (dash, consoles, floor,
  bulkhead) face the cabin.
- `make_cockpit.py`: the Blender script (background only:
  `blender.exe --background --factory-startup --python make_cockpit.py`). Builds everything from the numbers in it, exports
  the OBJ (Blender's exporter, Y up / -Z forward, so the file is in the space above), renders the three views into `work/`
  (a flat colour-per-part pass for outlines, a shaded pass, plus `check_glass_1.png` and `check_outside.png`), writes
  `work/keypoints.json`. `work/run.sh` runs it and `compare.py`.
- `compare.py` -> `compare_1.png`, `compare_2.png`, `compare_3.png`: top half the screenshot (brightened so the dark struts
  show) with the render's outlines in green and the frame tinted cyan; bottom half the shaded render over the screenshot.
- `calib.py` (HUD points read off the screenshots, and a rotation-only fit), `calib3.py` -> `calib3.json` (the cameras used),
  `tri.py` (rays, projection, triangulation), `measure.py` (frame corners against the screenshots), `pano.py` (all three
  views in one azimuth/elevation map), `crop.py` (magnified crops with a pixel ruler, for reading coordinates).

## The field of view (the spec's Q6, measured off the pictures, grade E)

**Vertical field of view about 61 degrees** on the 2000 x 821 pictures (focal length 697 px; horizontal 110 degrees), which is
**about 62 degrees over the user's full 3440 x 1440 screen** if the crop (3440 x 1412) took rows evenly off top and bottom.

How: the orange HUD panels are fixed in the cockpit, so the same HUD point in two views pins one focal length and each view's
head turn. 17 HUD points (letters, icons, the radar's centre, both upper panels) plus 4 frame corners were read off magnified
crops, each in two or three views, and fitted together (`calib3.py`). A head turn alone left 10 px of error with a pattern
(near points spread more than far ones): Elite's head look also moves the eye, so each turned view also gets a small
offset, and then everything fits to 2.9 px rms. The estimate moved between 57 and 61 degrees with the model (HUD points
alone, turn only: 57.7; HUD points with the eye's offset: 56.8; with the frame corners: 61.0). The frame corners' assumed
distances barely matter (halved: 59.8; doubled: 60.5). Letting views 2 and 3 roll as well lowers the error only from 2.9 to
2.8 px but pulls the answer to 54.6 degrees (the roll fitted is 2 and -4 degrees, at the edge of what was tried): the
pictures cannot tell roll from field of view, and Elite's head look has no roll, so the model keeps none. **Take it as
about 61, somewhere in 55-62.**
The image centre was assumed to be the picture's centre (freeing it made no real difference: 412 against 410.5).

## How each view's turn was matched

From `calib3.json` (yaw positive = turned left, pitch positive = up; the eye's offset in cm, X right, Y up, Z back):

| View | Yaw | Pitch | Eye offset |
|---|---|---|---|
| 1, straight ahead | 0 (fixed) | +2.3 | 0 (the origin) |
| 2, the left panel | +38.5 | -8.5 | (-0.5, +3.2, -3.6) |
| 3, the right panel | -37.7 | -4.1 | (+1.2, +3.8, -6.2) |

The offsets' size depends on one guess that fixes the scale: the radar hologram's centre is 0.75 m from the eye. Everything
in the model is in that scale (it puts the dashboard's HUD 0.6-0.8 m away and the side panels 0.6 m out). The renders use
exactly these cameras (Blender camera: vertical sensor fit, `angle_y` 61.0 degrees).

**The scale checked against the hull** (the eye renders look the same at any scale about the eye, so this is the only
check): in the top view `7.webp` the hull's front edge spans about 700 px for 22.2 m (0.032 m a px; its 14.4 m length gives
0.034). The canopy's clear glass, where the seat shows through, is about 40 px wide and 100 px long: **about 1.3 m by
3.2 m**, inside a dark surround about 3.7 m wide. The model's roof glass between the roof rails is 1.12 m wide and its
glass runs 2.6 m fore and aft (z -2.13 to +0.45): the same size within 15-25 %, so the scale was kept (a uniform x1.1-1.2
would fit the top view slightly better; it would change no comparison picture, only the eye offsets above in proportion).

## What was modelled, and from what

View 1 decides every direction; a depth was then chosen per piece to make a cockpit that holds together (only views 2 and 3,
whose eye has moved a few cm, see depth at all, and only weakly).

- **Frame** (dark struts, 0.045-0.10 m bevelled beams): the **crossbar** across the top of the windscreen (view 1 y 230-275,
  az +14 to -19 degrees; at z -1.9, triangulated from views 1 and 2 at about -2.0); at each end a junction where a
  **lower beam** sweeps down and back to a **knee** at eye height (left knee triangulated at (-0.58, 0.06, -1.06)) and a
  **side rail** runs back at eye height along each side (x +-0.60, seen in all three views, to the bulkhead at z +0.45);
  a **rising beam** from each junction up and back to an **arch overhead from side to side** just in front of the head
  (z -0.62, 0.9 m up), and two roof rails back to the bulkhead; the windscreen's lower frame (under the dash, out of
  sight), thin sill lips along the side windows, and the bulkhead behind the seat.
- **Dash**: its top edge (the coaming) is view 1's straight dark line at y = 576 across the whole width (el -11.6 degrees);
  it and the sills are kept on the plane through the eye that this line is, so it lines up whatever its depth (front edge at
  z -1.10, bending back to the sills at x +-0.62). Behind it a deck, a slope down to the radar's well with the radar's round
  housing (radius 0.2 m), two hologram pedestals where the left target box and the right ship hologram float, the face
  towards the pilot and the knee space under it; in front of it the nose slopes down to the windscreen's base (hidden).
- **Consoles**: from the sills (y about -0.10) in to the arm rests (x +-0.46, y -0.30), with a rounded arm rest roll along
  each inner edge, the throttle grip under the left hand and the stick under the right.
- **The pilot is not modelled.** Elite draws the commander: view 2's big ribbed cylinder under the left panel is the left
  forearm reaching for the throttle, and the white sleeve with a wrist band at the bottom of view 3 is the right forearm on
  the stick. Only what lies under them is modelled: the brown roll under the right arm (bottom of view 3) and the rounded
  end with buttons at the bottom of view 2 are the arm rests, which are the "top edges of the seat" the ticket means; they
  are the consoles' arm rest rolls here.
- **Seat**: cushion, back, head rest, arm rests, bolsters, behind and below the eye. Apart from the arm rests above, none of
  it shows in the three views.
- **Floor** at y -1.15, with the footwell under the dash.
- **Glass**: windscreen, front quarters, side windows (low and high), roof in front of and behind the arch. From the eye it
  covers every opening with no gap (`work/check_glass_1.png`); from outside, opaque (`work/check_outside_glass.png`,
  `check_front_glass.png`), it is one closed, boxy canopy with no panel folding through the cabin.

## How close each view is

Read off `compare_N.png` and `measure.py`:

- **View 1**: the struts, the junctions, the knees, the rails and the coaming line are placed from this view's pixels, so they
  sit on the screenshot's struts within about 5 px (the beams' widths were matched by eye: the right rail's dark band is
  thicker in the screenshot, 45 px against about 30). The left rising beam is the right one's mirror: in the screenshot it is
  dark on dark space and cannot be checked (a dark wedge behind the comms panel is consistent with it).
- **View 2**: the left junction within 3 px, the crossbar within about 15 px, the left rail within 10-15 px, the dash's top
  edge within about 15 px; the throttle grip sits at the end of the commander's left forearm.
- **View 3**: the crossbar and the right junction within about 20 px, the right rail and its knee within about 15 px, the
  dash's top edge within about 15 px. Along the crossbar the errors go opposite ways (the right junction lands 20 px low
  and right, the bar's step 17 px high and left): the render's bar comes out a little longer and more tilted than the
  screenshot's, which is what a slightly wrong field of view or a small roll would do (a roll was tried in the fit: see
  the field of view above). The rising beam's top is the worst: 44 px higher in the render than its highlight in the
  screenshot (a depth change cannot fix it: the views' eyes are too close together for parallax to matter; the turn is a
  little off there, or the highlight read is a different part of the beam).

## What was left out

- The HUD and holograms (step 4), the radar's stalks, every glowing panel.
- The dashboard's fine detail under the HUD (the radar housing's legs and the small lit stalks at the bottom of view 1, the
  round dial at the right end of the dash in view 3, buttons and greebles): the dash is a few flat planes and three cylinders.
- Anything no view shows: what is overhead (the views reach 28 degrees up at most), the seat, the rear of the cabin. Those
  are plausible guesses (the arch overhead, the roof rails, the bulkhead at z +0.45) sized to the hull (the canopy's top is
  0.95 m above the eye, under the hull's top 1.3 m up).
- The frame in the screenshots is slightly lopsided (the crossbar's middle is at x 1035, not 1000); the model keeps the
  screenshot's directions where they were seen (crossbar, lower beams, knees) and mirrors only what was not seen.
- Materials are flat colours; the uvs are a plain box projection for later texturing.
