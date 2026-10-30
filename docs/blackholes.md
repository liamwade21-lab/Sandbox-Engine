# Black holes, neutron stars and white dwarfs: Kerr-Newman lensing, accretion disks and jets

Code: `crates/se_engine/src/blackholes.rs` (`BlackHolesPlugin`), `blackhole_params.rs` (SE's disk
keys and defaults), shaders `assets/shaders/blackhole_lens.wesl` and
`assets/shaders/accretion_disk.wesl`. SE's own: `einstein.glsl`, `gr_geodesic.glh`,
`gr_intersection.glh`; CPU side FUN_140270900 (uniforms), FUN_140507690 (disk defaults),
FUN_14046c410 (`AccretionDisk` keys), FUN_1404d6710 (render sphere), FUN_1405045a0 (star set-up,
`KerrSpin` defaults), FUN_1404d4100 (draws every class-11 star with the GR shader).

SE's class word (body +0x1620) of these stars is class 11 (0x5800) with type < 12 a white dwarf,
12 a neutron star, 13 a black hole and 14 a wormhole (wormholes are not ported). SE draws the
first three with einstein.glsl in its `KERR_NEWMAN_METRIC` mode, neutron stars and white dwarfs
with the `STAR` define (their surface is traced too), instead of the ordinary star renderer.

## Approach

Everything is **one HDR post-process on the main camera** (`lens_pass`, a custom pass in
`Core3dSystems::EarlyPostProcess`, after all geometry and before bloom and tone mapping), SE's
einstein.glsl in its single-pass "COMPUTE + RENDER" mode. For each pixel near such an object the
camera ray is traced through the Kerr-Newman geometry; like SE's `EvaluateIntersections`, the
straight chord of every integration step is ray-marched through the disk and jet volume and
tested against the star's surface; the escaping ray is then looked up in the frame or the
environment map (`Skybox()`).

### CPU side (`update_lens`, `PostUpdate`, after `UniverseSet::Camera`)

* All catalog stars of kind `BlackHole`, `NeutronStar` and `WhiteDwarf` are collected once
  (re-collected when the star count changes, e.g. materialised procedural stars). Black holes
  have no `SystemBody`; this plugin evaluates them from the catalog + `Ephemeris`.
* `Rg` = Schwarzschild radius, 2.95325 km per solar mass (SE: 8.87005e-6 km per Earth mass,
  FUN_1404dcd30); 10 Msun if the catalog has no mass. The unit of every length the shader sees is
  SE's gravRadius: `Rg`, but the star's radius `R` for a white dwarf, whose metric mass is then
  scaled by KerrM = Rg / R (`starRadiusScale`, DiskParams5.w; FUN_140270900 `local_9c8`).
  "Units" below means that unit.
* Metric (MetricParams): `KerrSpin` and `KerrCharge` from the catalog. SE's `KerrSpin` defaults
  (FUN_1405045a0): a hole N(0.5, 0.15) cut to 0..0.9999 up to 1000 Msun, 0.5 sqrt(u) + 0.4999
  above; a neutron star N(0.2, 0.05); a white dwarf (0.2 u)^2. `KerrCharge` defaults to 0 (only
  Mini-Sky gives it, as 0).
* Star surface (StarParams, neutron stars and white dwarfs): temperature `Teff`, raised to the
  disk's when the disk is hotter (FUN_140507690), at most 60000 K; radius R / unit; equator speed
  `R 2 pi / (RotationPeriod x 6378.14 x 86400) / 299792448` (SE's literal formula, ~1e-9 c, so
  its Doppler term does nothing); starTime = frac(t / (1000 sqrt(R/km) s)) x 256; starRotAng =
  the prime meridian's angle about the axis.
* Disk and jets: `params::of_star` = SE's key parser (FUN_14046c410: AU keys after km keys,
  `Thickness` sets both, `Exposure` = `Brightness`, `Lum`, `LumBol`, kg/s accretion) and SE's
  defaults (FUN_140507690, black hole branch): a hole without the tag gets a disk with
  probability 0.2, holes over 400 Msun always; temperature N(6000, 700) K (stellar),
  N(4000, 600) below 1e5 Msun, N(10000, 1500) above; `InnerRadius` 1.01 R, `OuterRadius`
  30 x inner (no disk at or below 2000 K); `InnerThickness` (0.25..1) x 0.2 R, `OuterThickness`
  inner x (18 h + 2) and at least 0.55 R, h = clamp((T - 3000)/7000)^2; accretion rate, density,
  `Luminosity`, `LuminosityBol` from SE's formulas; `TwistMagn` 2500, `DetailScaleR` 6,
  `DetailScaleV` 6 - 5h, octave distortion (0.5, 0.35, 0.2), octave scale 0.5, noise contrast 1,
  temperature contrast 0.15, outer spin 0.04; jets start 1.5 R, end 5 x start, length 30 x
  start, temperatures T and 0.75 T, velocity 0.1 + 0.75h, density h^3 1e-4 x disk density,
  thickness 0.3, distortion 0.4. `KerrSpin` default: N(0.5, 0.15) cut to 0..0.9999 up to
  1000 Msun, 0.5 sqrt(u) + 0.4999 above (FUN_1405045a0). SE draws from its thread RNG seeded per
  star record; the port seeds from the name, so the draws differ.
* Neutron stars and white dwarfs go through the same FUN_140507690 (`bVar4` white dwarf, `bVar3`
  black hole): without the tag 20% get a disk; the default temperature is N(30000, 3000) K (a
  stellar hole's N(6000, 700)); the geometry follows the star's radius R (inner edge 1.01 R,
  thickness, jets from 1.5 R, a white dwarf's from 3 R ending at 2.5 x that) and the alpha-disk
  formulas its Rg; a white dwarf uses x = R / Rg where a hole has 4.32 and 3.94453, takes its disk
  temperature from the accretion rate (FUN_1404dfdc0), its thermal light from 2 R, and has no
  density cap. With SE's formulas a neutron star's default disk is thin (density ~1e-5 g/cm^3)
  and faint.
* Uniforms (FUN_140270900, default "hdr" mode): render sphere (FUN_1404d6710) =
  `DistortionRadiusKm` / `DistortionRadiusAU` if given, else max(`BlackHoleWarpRadius` Rg, R)
  (3 R for a white dwarf), widened to the outer radius and the jet length; gravS = unit / sphere
  radius (SE integrates densities per sphere unit); diskDensity = `Density` x unit(km) x 6.4e7;
  temperatures clamped to 80000 K; jet brightness `JetBrightness` x hdrAccretionJetBright /
  hdrAccretionDiskBright; LightColor = normalised Planck colour at 0.8 T x `Brightness` x
  `LightingBright` x 0.01; volumBright = hdrAccretionDiskBright x 10^(6984.51/T + 1.22651 -
  0.803771 ln T) x 1 / (1 - (1 - min(diskDensity 1e-6, 1))^sqrt2) x `Brightness`^-0.8 with T the
  hotter of the disk and the star's surface (without a disk only the temperature factor); warpStep
  = 0.2 x 10^(1 - `WarpPrecision`); march step = warpStep x min(1, max thickness); disk animation
  = frac(t / (135 x 6 pi Rg / (c / sqrt2))) (FUN_1404e5340), for a white dwarf 135 Kepler periods
  at the inner edge around one solar mass. Config keys read from main-def.cfg:
  hdrAccretionDiskBright, hdrAccretionJetBright, BlackHoleWarpRadius, WarpPrecision,
  EnvmapDimensions.
* Objects closer than 2 pc (catalog position) are tested every frame; one is lensed when 1.45 x its
  render sphere's angular radius (SE draws the GR impostor over that sphere; nothing changes
  outside it) is >= 1.5 px, and the camera is outside the horizon (1.5 Rg) or the star. Up to 4,
  largest on screen first. With none in range the `BlackHoleLens` component is removed and the
  environment cameras are switched off, so nothing is spent.
* The disk frame's x axis is fixed in the world (it used to turn with the camera's up vector).
* While a neutron star or white dwarf is lensed its ordinary star sphere and corona are hidden
  (`hide_lensed_stars`, after the systems that show them; SE draws these stars only with the GR
  shader and its glow after it). Here anything in the frame is lensed, and a neutron star's
  glow and flare turned into a bright ring around it, so a neutron star's point sprite is hidden
  too once the surface is half a pixel wide and the lens flares leave it out (lensflare.rs reads
  `LensedHoles`, which lists holes and neutron stars). A white dwarf bends light by only Rg / R
  and keeps both.
* `SE_NO_BLACKHOLES=1` disables the plugin; `SE_BH_DEBUG=1` logs the first hole's uniforms;
  `SE_BH_ENVTEST=1` shows the environment map instead of the frame; `SE_BH_KERR="a,q"` sets every
  lensed object's KerrSpin and KerrCharge, `SE_BH_EDGE=1` turns every spin axis to the screen's up
  direction (an edge-on view), both for tests.

### Lookup textures

* SE's PlanckFunction (FUN_1401a60d0, `planck_table`): 4096 entries, T = 1000 exp(i/4096/0.188
  - 0.7) (497 K .. 101 kK), the Planck spectrum at 380..780 nm in 5 nm steps against the CIE 1931
  colour matching functions (Wyman-Sloan-Shirley fit here, SE has the table), relative to the
  555 nm radiance of a 5700 K body, through the sRGB matrix (PlanckColorSystem 1), raised by
  1000^(1 - (T - 500)/1500) below 2000 K. Read as `texture(PlanckFunction, log(T/1000) x 0.188 +
  0.1316)`.
* SE's NoiseIqMap: 16 x 16 random R, B with G, A the R, B of the texel (-11, -5) away (iq's
  layered noise), repeat sampling (`noise_texture`).

### Light transport (blackhole_lens.wesl, accretion_disk.wesl)

* Geodesic (gr_geodesic.glh `TraceGeodesic`, `KERR_NEWMAN_METRIC`): the Kerr-Newman metric in
  Kerr-Schild coordinates, g = eta + f l l with l = (1, k), k = ((r x - a y) / (r^2 + a^2),
  (r y + a x) / (r^2 + a^2), z / r), f = r^2 (2 KerrM r - Q^2) / (r^4 + a^2 z^2) (`KerrGetR2`,
  `KerrGetK`), in units of M = unit / 2 with the spin axis as z (the disk frame's y). The ray is
  straight up to SE's integration sphere, radius clamp(2 |camera|, 100, 300) M but at most
  invGravRadius (the render sphere's radius in units); inside it the Hamiltonian is integrated
  with SE's explicit steps (p += forward-difference gradient of qt.G qt at fixed qt = Ginv p,
  eps 0.02 |q|; q += 2 qt dt), step (warpStep 0.8333 + 0.4) clamp(1/|p|, 0.1, 4) mix(0.06, 20,
  (r - 1) / 120) x dt_amplify, shortened by 0.05 + 0.95 r^3 inside r = 1; r < 1.1 is captured
  unless KerrA >= 1 (a naked singularity, then 1500 instead of 256 steps). Leaving the sphere,
  the direction is interpolated to the crossing; the bent direction is blended into the
  unbent one by smoothstep(sphere, 0, closest approach), so rays that only graze the sphere are
  barely bent. This gives the flattened, shifted (D-shaped) shadow of a spinning hole seen
  edge-on and frame dragging of the lensed sky.
* Star surface (`StarSurface`, STAR): on each chord after the volume, the first crossing of the
  sphere of radius R; gravitational shift sqrt(starR (D - 1) / (D (starR - 1))) with starR = R /
  KerrM, the equator's Doppler shift, temperature x (0.7 + 0.3 Fbm) with the spots drifting by
  starTime, 1.25 x hotter poles, x (1 + z)^-1.25 x 0.75; Planck emission x volumBright. The ray
  stops there (SE keeps integrating with zero transmission; the result is the same).
* Volume (gr_intersection.glh line by line, in units): each straight part and geodesic chord is
  marched on a lattice of the step length (blue-noise jitter replaced by interleaved gradient
  noise), the emission time from SE's timeSegment (Kerr-Schild time / (16000 KerrM)), empty
  space skipped with SE's bounding SDF (disk cylinder + jet cones). `DiskProperties`: alpha-disk temperature
  5.1293 f^0.3 r^-0.65 (f = 1 - sqrt(3/r), r = (1.25 + 0.8 KerrSpin) rho), density
  r^-0.65 linearstep(1, 3, r) x vertical/inner/outer profiles, Keplerian v = (2(r - 1))^-1/2,
  shell noise (`ShellFbm`, 3 octaves, octave LOD by path length, Keplerian rotation of each shell
  quantised to loop seamlessly), gradient shadows beyond r = 12. `JetProperties`: rotating,
  noise-distorted capped cylinders. `VolumeStep`: illumination density x (1 +/- shadow) x
  LightColor / (100 + r^2), temperature (2e14 |illum| + T^4)^1/4, Planck emission at T / (1 + z)
  with SE's (1 + z) = rho (D - 1) / (D (rho - 1)) x (1 + n.v) / sqrt(1 - v^2), extinction with
  optical density (0.7, 0.85, 1.0) per channel, stop at optical depth 12, at most 512 march / 256
  dense steps.
* Sky (`Skybox()`): the frame where the escaping ray lands on screen, SE's screen-edge blend
  zone, and the environment map beyond it and for every ray behind the camera; within 3..10 units
  of the hole the environment map takes over entirely (blendFactor = smoothstep(3, 10, D)).
  Then `SkyBlueShift`: the sky seen from deep in the well is blue-shifted and brightened by
  (1 - KerrM/D)^-2. Captured rays are black. Result: sky (or the star's surface) x
  transmission(sigma) + emission x volumBright.
* 4 rotated-grid background samples for pixels with impact parameter < 14 units (shadow edge,
  photon ring, star limb); the volume is traced on the centre ray only. The background
  sub-samples use a separate volume-free trace (keeping the shader small matters: the inlined
  volume code made one view 200x slower); each gives the sky, the star's surface or black.

### Environment map (SE's EnvMap, seEnvMapManager)

Six cameras (`EnvFaceCam`, orders -1..-6, HDR, no tone mapping, no MSAA) render the scene into
six `EnvmapDimensions`-sized cube faces along the world axes from the main camera's position.
They are spawned on first use, render all six faces when the map is (re)started and one face per
frame after that, and are switched off while no hole is lensed. The map is rebuilt when the
camera has moved by 10% of its distance to the hole. The lens flare quads are on render layer 1
(`MAIN_VIEW_ONLY`), which only the main camera sees.

### Seen from afar

A black hole whose `AccretionDisk` has a catalogued `Luminosity` is a point in the catalog
star field (`disk_light`, stars.rs): absolute magnitude 4.83 - 2.5 log10(L), blackbody colour at
the disk temperature (SE adds the disk luminosity to its system's light, FUN_14050ed10). The
star fields no longer hide the active system's point when that system has no bodies (a lone
hole).

## Test commands

```
SE_NO_MUSIC=1 spaceengine-rs.exe --run "date 2026-10-26 12:00; pause; goto Cygnus X-1 A 400; wait 10; move 0 4500000 0; look Cygnus X-1 A; wait 90" --screenshot out.png --frames 200
```

Others: `goto XTE J1859+226 A 300` (all-default disk, camera inside it), `goto Sagittarius A*
300000000` + `move 0 1000000000000 0` (catalog "realistic" disk), `goto Gaia BH1 B 600` (lensing
and the environment map), `search 300 bh` then `rs NAME` (procedural holes, e.g.
`RS 10-581-646-511-10000`), `galactic 1000; look Sagittarius A*; fov 2` (Sgr A* as a point).
Kerr shadow: `SE_BH_KERR="0.998,0" SE_BH_EDGE=1` with `goto Gaia BH1 B 1500; wait 100; fov 12`
(D-shaped, shifted to one side; `"0,0"` gives the round Schwarzschild shadow). Neutron stars:
`goto Crab Pulsar 1500` / `120`, `goto Hercules X-1 300` (default disk); white dwarf with a disk:
`goto U Gem B 1500000` (catalog `Density 5000`: opaque disk and jets).

Cost (RTX 5080, shared): about +1.2 ms per frame with a disk filling the screen, +0.6 ms for
lensing with the environment map; nothing without a hole in range.

## Limitations

* Like SE, light is bent only inside its integration sphere (at most 150 units, half the render
  sphere), faded out towards its edge: a distant hole shows no Einstein ring of the sky beyond
  that (the Schwarzschild trace this replaced bent rays at any distance).
* A neutron star's or white dwarf's point sprite, corona and lens flare are in the frame before
  the lens, so they are hidden or left out while it is lensed (see above) instead of being drawn
  after it like SE's glow; a star under half a pixel keeps its sprite, whose lensed core can
  show as a faint ring inside the glare.
* Catalog `Class "Neutron"` (PSR J1719-1438, PSR J1300+1240) is read by the port's class parser
  as a carbon star, so those are not drawn as neutron stars.
* Single full-resolution pass, one volume sample per pixel; SE's "high" quality computes the
  disk at low resolution and upsamples it bicubically (smoother, less aliasing of the finest
  noise octave close to the inner edge).
* Particle jets (`accretion_jet.glsl`, FUN_140483370) are not ported: SE draws them only for
  neutron stars and black holes that carry a `CometTail { Particles N }` block, which no catalog
  object has.
* Environment map: other renderers size sprites in pixels of the view (galaxy model sprites,
  star points), so the 90-degree faces differ slightly from the main view and faint seams can
  appear at face edges in the lensed sky. Content between the camera and the hole is lensed as
  if it were at infinity (SE does the same outside its WARP mode).
* Several holes in view: each pixel is lensed by the single hole whose region contains it most
  deeply; overlapping regions are not composed.
* `disk_light` uses only catalogued `Luminosity`; the defaults derived from the accretion rate
  are not shown as points (how SE feeds them to the distant point is not decoded).
* `Precession`, `PrecessionPeriod`, `NutationPeriod` (SS 433) are not read by SE either.
