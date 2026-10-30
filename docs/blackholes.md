# Black holes: lensing, volumetric accretion disks and jets

Code: `crates/se_engine/src/blackholes.rs` (`BlackHolesPlugin`), `blackhole_params.rs` (SE's disk
keys and defaults), shaders `assets/shaders/blackhole_lens.wesl` and
`assets/shaders/accretion_disk.wesl`. SE's own: `einstein.glsl`, `gr_geodesic.glh`,
`gr_intersection.glh`; CPU side FUN_140270900 (uniforms), FUN_140507690 (disk defaults),
FUN_14046c410 (`AccretionDisk` keys), FUN_1404d6710 (render sphere).

## Approach

Everything is **one HDR post-process on the main camera** (`lens_pass`, a custom pass in
`Core3dSystems::EarlyPostProcess`, after all geometry and before bloom and tone mapping), SE's
einstein.glsl in its single-pass "COMPUTE + RENDER" mode. For each pixel near a hole the camera
ray is traced through the Schwarzschild geometry; like SE's `EvaluateIntersections`, the straight
chord of every integration step is ray-marched through the disk and jet volume; the escaping ray
is then looked up in the frame or the environment map (`Skybox()`).

### CPU side (`update_lens`, `PostUpdate`, after `UniverseSet::Camera`)

* All catalog stars with `class.kind == StarKind::BlackHole` are collected once (re-collected
  when the star count changes, e.g. materialised procedural stars). Black holes have no
  `SystemBody`; this plugin evaluates them from the catalog + `Ephemeris`.
* `Rg` = Schwarzschild radius, 2.95325 km per solar mass (SE: 8.87005e-6 km per Earth mass,
  FUN_1404dcd30); 10 Msun if the catalog has no mass. Lengths below are in Rg.
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
* Uniforms (FUN_140270900, default "hdr" mode): render sphere = max(`BlackHoleWarpRadius` Rg,
  outer radius, jet length) (FUN_1404d6710), gravS = Rg / sphere radius (SE integrates
  densities per sphere unit); diskDensity = `Density` x Rg(km) x 6.4e7; temperatures clamped to
  80000 K; jet brightness `JetBrightness` x hdrAccretionJetBright / hdrAccretionDiskBright;
  LightColor = normalised Planck colour at 0.8 T x `Brightness` x `LightingBright` x 0.01;
  volumBright = hdrAccretionDiskBright x 10^(6984.51/T + 1.22651 - 0.803771 ln T) x
  1 / (1 - (1 - min(diskDensity 1e-6, 1))^sqrt2) x `Brightness`^-0.8; warpStep =
  0.2 x 10^(1 - `WarpPrecision`); march step = warpStep x min(1, max thickness); disk animation
  = frac(t / (135 x 6 pi Rg / (c / sqrt2))) (FUN_1404e5340). Config keys read from main-def.cfg:
  hdrAccretionDiskBright, hdrAccretionJetBright, BlackHoleWarpRadius, WarpPrecision,
  EnvmapDimensions.
* Holes closer than 2 pc (catalog position) are tested every frame; a hole is lensed when its
  region (several Einstein radii, the shadow, the disk and jet volume) is >= 1.5 px. Up to 4
  holes, largest on screen first. With none in range the `BlackHoleLens` component is removed
  and the environment cameras are switched off, so nothing is spent.
* `SE_NO_BLACKHOLES=1` disables the plugin; `SE_BH_DEBUG=1` logs the first hole's uniforms;
  `SE_BH_ENVTEST=1` shows the environment map instead of the frame.

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

* Geodesic: in the plane of observer, hole and ray (Schwarzschild, r_s = 1),
  `u'' + u = 3/2 u^2`, RK4 in the polar angle; steps 0.3 / 0.12 / 0.05 rad by u. `u >= 1`:
  captured. Outbound beyond twice the volume's reach: the remaining sweep is added analytically
  and the direction at infinity looked up.
* Volume (gr_intersection.glh line by line, in Rg): each RK4 chord is marched on a lattice of
  the step length (blue-noise jitter replaced by interleaved gradient noise), empty space skipped
  with SE's bounding SDF (disk cylinder + jet cones). `DiskProperties`: alpha-disk temperature
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
  zone, and the environment map beyond it and for every ray behind the camera; within 3..10 Rg
  of the hole the environment map takes over entirely (blendFactor = smoothstep(3, 10, D)).
  Then `SkyBlueShift`: the sky seen from deep in the well is blue-shifted and brightened by
  (1 - 1/D)^-2. Captured rays are black. Result: sky x transmission(sigma) + emission x
  volumBright.
* 4 rotated-grid background samples for pixels with impact parameter < 14 Rg (shadow edge,
  photon ring); the volume is traced on the centre ray only. The background sub-samples use a
  separate volume-free trace (keeping the shader small matters: the inlined volume code made
  one view 200x slower).

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

Cost (RTX 5080, shared): about +1.2 ms per frame with a disk filling the screen, +0.6 ms for
lensing with the environment map; nothing without a hole in range.

## Limitations

* Schwarzschild geometry: `KerrSpin` only enters the disk physics (r scaling) and SE's Kerr-
  Newman metric (flattened, shifted shadow, frame dragging, `KerrCharge`) is not ported.
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
