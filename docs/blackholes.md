# Black holes: lensing and accretion disks

Code: `crates/se_engine/src/blackholes.rs` (`BlackHolesPlugin`),
shaders `assets/shaders/blackhole_lens.wesl` and `assets/shaders/accretion_disk.wesl`.

## Approach

Everything is **one HDR post-process on the main camera**, built on Bevy's
`FullscreenMaterial` (`bevy::core_pipeline::fullscreen_material`). It runs in
`Core3dSystems::EarlyPostProcess`, i.e. after all opaque/transparent geometry (stars,
planets, galaxies, overlays) and *before* bloom and tone mapping, so the photon ring and the
beamed side of the disk bloom naturally. The pass reads the finished scene colour and writes
the lensed result.

There is no disk mesh: the disk is raytraced in the same pass, so it is lensed by the same
geometry as the background ("Interstellar" look: the far side of the disk appears above and
below the shadow, the near side passes in front of it).

### CPU side (`update_lens`, `PostUpdate`, after `UniverseSet::Camera`)

* All catalog stars with `class.kind == StarKind::BlackHole` (class `X`, non barycentre) are
  collected once (re-collected when the star count changes, e.g. procedural stars).
  Black holes have `radius_km == 0`, so `system.rs` never spawns a `SystemBody` for them; this
  plugin therefore evaluates them directly from the catalog + `Ephemeris` (orbital position,
  rotation model) and does not depend on `ActiveSystem`.
* Schwarzschild radius `r_s = 2GM/c^2 = 2.95325 km * M/Msun` from `mass_kg` (10 Msun if the
  catalog has none).
* Holes closer than 2 pc (catalog position) are tested every frame. A hole is kept if the
  angular size of its effect region is >= 1.5 px (so Sgr A* is only active within ~a few
  hundred AU, a 20 Msun hole within a few AU, etc.). Up to 4 holes, largest on screen first.
* Per hole the shader gets (view space): unit direction, distance in `r_s`, disk normal,
  disk reference axis, inner/outer radius (`r_s`), brightness, colour temperature, turbulence
  contrast, and the angular radius of its influence region
  (`max(16 theta_E, shadow, disk)`, `theta_E = sqrt(2 r_s / D)`).
* When nothing is in range the `BlackHoleLens` component is removed from the camera, so the
  pass costs nothing.
* `SE_NO_BLACKHOLES=1` disables the plugin (debugging).

### Disk parameters

From `stars[i].block.sub("AccretionDisk")` via `AccretionDiskParams` (a hole without the
block, or with `NoAccretionDisk`, gets lensing only):

| catalog key | use |
|---|---|
| `InnerRadius` (AU) / `InnerRadiusKm` | inner edge, clamped to `max(ISCO(KerrSpin), 2 r_s)` |
| `OuterRadius` (AU) / `OuterRadiusKm` | outer edge; default 60 r_s |
| `Temperature` (K) | compressed to a display colour temperature `4000*(T/1e4)^0.3`, 3500..14000 K (default 1e5 K) |
| `Brightness` | `Brightness^0.1` clamped to 0.35..2 (catalog values range 1e-6..2500) |
| `DiskNoiseContrast` | turbulence contrast (default 0.7) |
| `KerrSpin` | only the ISCO radius (Bardeen formula) |

Disk plane: equatorial plane of the hole's rotation model (`Ephemeris::orientation * +Y`,
this is how Sgr A* gets `Obliquity 24 / EqAscendNode 100`). Holes without a rotation model
(pole = ecliptic north) get a deterministic tilt of 30-70 degrees from a hash of the name.

### Light transport (blackhole_lens.wesl)

For each pixel of a hole's region the camera ray is traced **backwards** in the plane spanned
by observer, hole and ray (Schwarzschild, units `r_s = 1`):

```
u = 1/r,   u'' + u = 3/2 u^2,    (u')^2 + u^2 (1 - u) = 1/b^2   (conserved)
```

* Start: `u0 = 1/D`, `u' = u0 sqrt(1-u0) cot(alpha)` (alpha = angle between ray and the
  direction to the hole), RK4 in the polar angle phi.
* Step size: 0.3 rad for u < 0.02, 0.12 for u < 0.06, 0.05 near the hole; the step is
  shortened to land exactly on disk-plane crossings (phi_c + k pi).
* `u >= 1` -> captured (the black shadow, critical impact parameter 3 sqrt(3)/2 r_s).
* Outbound with `u < 0.5/r_out` (at least 2 outer radii away): stop and add the remaining
  sweep to infinity analytically, `asin(u/sqrt(K))` with a first-order correction of the
  `u^3` term (`K = u'^2 + u^2(1-u)`); the direction at infinity is then
  `cos(phi_end) e1 + sin(phi_end) e2` and is looked up in the scene colour (bilinear).
  Lookups that land off screen are mirror-reflected and faded to black beyond 2.4x the screen
  half-size or behind the camera.
* Each disk-plane crossing with `r_in < r < r_out` adds emission:
  * temperature profile `T ~ x^(3/4) (1 - sqrt(x))^(1/4)`, `x = r_in/r`, normalised to its peak
    at 1.36 r_in (Novikov-Thorne-like), emissivity ~ T^2 (compressed for display);
  * frequency shift of Keplerian gas, exact for circular geodesics:
    `g = sqrt(1 - 3/(2r)) / (1 - Omega L_z/E)`, `Omega = sqrt(1/(2 r^3))`, `L_z/E` from the
    photon's impact parameter and orbital-plane orientation. Surface brightness scales as
    `g^3`, colour temperature as `g` (blackbody colour), so the approaching side is brighter
    and bluer, the receding side dimmer and redder, and the inner edge is gravitationally
    redshifted;
  * animated turbulence (3 octaves of periodic value noise in azimuth x ln r) sheared by the
    Keplerian angular velocity (slowed down 6x for readability);
  * opacity `<= 0.92`, thinning outwards, so secondary images show through.
  Emission is accumulated front to back with the remaining transmission; the lensed
  background is attenuated by it.
* 4 rotated-grid samples for pixels with impact parameter < 9 r_s (shadow edge, photon ring,
  inner disk), 1 sample elsewhere.

Cost (RTX 5080): 150-270 fps with the hole filling a third of the screen; the worst case seen
(camera at 4.6 r_s, hole + disk covering the whole screen) is ~65 fps. Rays far from the hole
finish in about 10 integration steps.

## Test commands

```
SE_NO_MUSIC=1 ./target/debug/spaceengine-rs.exe --run \
  "date 2026-10-26 12:00; pause; goto Cygnus X-1 A 400; wait 10; move 0 4500000 0; look Cygnus X-1 A; wait 90" \
  --screenshot out.png --frames 200
```

(`goto "Cygnus X-1"` lands on the barycentre, ~0.1 AU from the hole; use the `A` component.
`goto` + `move` + `look` is an easy way to pick the inclination. `--frames 200`: with fewer
frames the screenshot of heavy views can be taken before the scene has settled.)
Other targets: `Sagittarius A*` (disk, distance in km: try `300000000` then `move` 3e12 m
sideways), `Gaia BH3 B` / `Gaia BH1 B` (no disk: Einstein ring only).

## Limitations

* **Screen-space lensing**: only what is already in the frame can be lensed. Objects between
  the camera and the hole (e.g. the companion star of a binary when it is in front) are lensed
  as if they were at infinity, and several catalog companions sit at the same position as the
  hole, so their sprite shows up as an Einstein ring. Content behind the camera or outside the
  view is replaced by mirrored screen content that fades out; the deep interior of the
  Einstein ring therefore shows a darkened, reflected sky rather than the true rear sky.
* Schwarzschild (non-rotating) geometry for the lensing; `KerrSpin` only moves the ISCO.
  No frame dragging, no flattened shadow, no jets (`Jet*` keys ignored), no `Twist*`,
  `Precession`/`Nutation` (SS 433), `DiskOuterSpin`.
* The disk is infinitely thin (thickness keys ignored) and its emission is an artistic
  mapping of the physical temperature/brightness catalog values, not a radiative transfer
  solution. Disk plane is the equatorial plane of the rotation model, not the orbital plane
  of a binary companion.
* Per-ray precision is f32: the effect is only active while its region is >= 1.5 px (about
  `D < 6e5 r_s`), which keeps relative angles well inside f32 precision.
* Rays that circle the photon sphere more than ~1.3 times (220 steps) are treated as
  captured, so higher-order photon-ring images are absent.
* Several holes in view: each pixel is lensed by the single hole whose region contains it
  most deeply; overlapping regions are not composed.
* Very large magnification of 1-px-wide features (thin overlay lines, tiny sprites) near the
  Einstein ring shows texel staircases, because the screen has no more resolution than the
  frame buffer.
