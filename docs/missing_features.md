# What the port is missing or does differently from SpaceEngine 0.991

An audit of the Rust/Bevy port against SpaceEngine 0.991.49 (2026-10-28). Nothing in the port was changed for it.

**How it was made.** SpaceEngine was split into ten areas (tickets in `.audit/tickets/`). One auditor per area listed
the area's features from SE's own files (the decrypted shaders in `re/se_shaders`, `config/main-def.cfg` keys,
`eng-gui.cfg` strings, exe strings and decompiled functions from the Ghidra project, catalog and script keys) and
checked the port for each, running it where reading was not enough. Every reported gap then went to a checker
whose job was to prove it wrong (find the port code, or show SE does not do it); only gaps that survived are
listed below, some with corrected wording. Out of scope: VR, the editors, online services.

**Limits.** No gap was confirmed by playing SpaceEngine itself (that needs your screen); SE behaviour comes from
its code, shaders and data. Shader-level differences are exact; visual weight is judged from the code, not side by
side. Effort is a rough estimate.

## The two reported bugs

### Regor Ab (gamma Velorum) invisible when selected from 0.25 AU

**Fixed 2026-10-28.** The active system is now the one whose real star is nearest (components measured at their
ephemeris positions, `system.rs` `nearest_catalog_star`); the star fields hide only the active system's own point,
by position (`StarParams::hide`); Pozzo 1 gets SE's luminosity. Regor Ab shows as a 40-degree disc from 0.25 AU and
GAM Vel stays the active system on a fly-through (`.audit/work/fix/regor_025.png`, `fix/gvel/`). The original
diagnosis follows.

Reproduced (`goto Regor Ab 37400000`): the log shows `entered system GAM Vel` and at once
`entered system Pozzo 1 #443`, and the selection ticks frame empty sky. The star itself renders fine: from 600 AU
off the system's barycentre, with GAM Vel the active system, `look Regor Ab`, `look Regor Aa` and `look Regor Ba` all
show the stars with glare and halo. Three things combine:

1. **The nearest-system test measures to the barycentre.** `select_active_system`
   (`crates/se_engine/src/system.rs:176-214`) compares catalog stars by `pos_pc`, and a component without its own
   RA/Dec gets its root's position (`crates/se_core/src/catalog.rs:704-723`). Regor A orbits the GAM Vel barycentre
   at 2863 AU (`planets/Stars-bin.sc`, `Barycenter "Regor A"` SemiMajorAxis 2863.07), so at Regor Ab the port puts
   the nearest catalog star 0.0139 pc away. A generated cluster member 1416 AU (0.0069 pc) away beats 0.9 times
   that (`system.rs:196`), becomes the active system, and every GAM Vel star entity is despawned
   (`system.rs:222-224`).
2. **Nearby star points are hidden on the assumption that the active system draws them.** Every star point within
   `LOCAL_HIDE_PC` = 0.02 pc is hidden and fades in up to 0.06 pc (`crates/se_engine/src/stars.rs:22`,
   `assets/shaders/stars.wesl:100-103`). GAM Vel's point, at 0.0139 pc, is hidden, so nothing at all is drawn.
   SE hides only the barycentre pseudo-star (`star_sprite.glsl:125`) and keeps drawing every star (gap 01-005).
3. **Pozzo 1 is overpopulated.** Pozzo 1 is centred on gamma Vel (`clusters/Open.sc`: RA 8.1589 h, Dec -47.337,
   336 pc, radius 0.88 pc, no magnitude). The port gives clusters without a magnitude AbsMagn -5
   (`catalog.rs:637-642`) and fills Pozzo 1 to its 3000-star cap (`clusters.rs:381-385`), concentrated on Regor.
   SE's rule gives it about 45 L_sun, a handful of members (`FUN_14047fcb0`, `ClusterCatMaxAppMagn` 8.1; gap 02-005).

Fix: measure system distance to the components' real (ephemeris) positions or keep the active system while the
camera is inside its extent; stop hiding points the active system does not draw (or keep several nearby systems);
follow SE's cluster luminosity rule. Effort M. Runs: `.audit/work/main/regor_025.png`, `look/row.png`, `gvel/`.

### Cluster sprites near Pozzo 1 fade out instead of turning into real stars

**Fixed 2026-10-28.** Star points no longer fade near the camera (see above); every cluster, catalog or procedural,
is SE's star-like particle fading out from 1 to 100 px while members appear from 8 px of radius
(`.audit/work/fix/clusters_grid.png`). The original diagnosis follows.

Two port mechanisms make cluster light disappear on approach where SE resolves it into stars:

1. **Member points fade and only one star becomes real.** Catalog-cluster members (and field stars) fade out
   between 0.06 and 0.02 pc from the camera (same `LOCAL_HIDE_PC` rule, `clusters.rs:127`), and only the single
   nearest candidate is turned into a real star (`system.rs:193-214`, `clusters.rs:240-279`). Every other point you
   fly up to simply fades away. Inside the port's overpopulated Pozzo 1 the active system flips between members
   every few frames (`#899, #94, #1003, #1689, #762, ...` in one fly-through) and gamma Vel's barycentre competes too.
   In SE every member is an octree star (`RSC ...`, `FUN_14053a760`) drawn as a point and, close up, as a star.
2. **Procedural `RC` clusters keep a glowing disc.** Their far sprite is a soft disc of constant surface brightness
   (`galaxy_far.wesl` ptype 3) that never fades with size: it swells into a large white glow over the member stars
   (run `rc/grid.png` at 15, 6 and 2 pc) and is cut off abruptly once the camera is inside 1.02 radii
   (`galaxy_far.wesl:146`). Members are only generated inside max(300 pc, 60 radii), three promotions per 10 frames,
   so they can be absent for a while. SE fades the cluster particle out from 1 px to 100 px of apparent radius
   (`star_point.glsl` `smoothstep(Param2.x, 1, RadiusPix)`, Param2.x = 100 x height/1080) while member stars appear
   from 8 px (`FUN_140499f30`, `FUN_1404a09a0`), so the glow hands over to stars (gap 02-002).

Catalog clusters have no far sprite in the port at all (gap 02-001), so near Pozzo 1 the fading "sprites" are member
points (1) and procedural clusters (2). Effort: (1) M, shared with the Regor fix; (2) S.

## Top 15, in the suggested order to fix them

Ranked by how much of SE's look and feel each restores per day of work, bugs first. Visibility (v) and effort as in
the list below.

| # | Fix | Gaps | v | Effort |
|---|---|---|---|---|
| 1 | **Fixed 2026-10-28.** Active system and nearby stars: measure to the components' real positions, keep the system while inside it, stop hiding star points that no active system draws (the Regor Ab bug and the fading member points) | reported bug 1, 2a; 01-005 | 4 | M |
| 2 | **Fixed 2026-10-28.** Clusters as SE builds them: its luminosity and age defaults (Pozzo 1 about 45 L_sun, old globulars), a far particle for catalog clusters, members from 8 px, procedural cluster glow faded out by 100 px | reported bug 2b; 02-005, 02-004, 02-001, 02-002 | 4 | S+M |
| 3 | **Fixed 2026-10-28.** Vega and Altair stall the port (seconds per frame, about 11 GB) on system entry | 01-004 | 4 | M |
| 4 | **Fixed 2026-10-28.** First-impression defaults: orbits, labels and markers off, FOV 45, time steps 1-3-10-30, Go to arrives at 4 radii | 08-004, 07-001, 07-002, 07-003 | 4-5 | S |
| 5 | **Fixed 2026-10-29.** Star points: SE's Gaussian core plus flare wing, linear brightness with soft compression, dim-star greying and colour saturation (flare wing and StarColorShift left out) | 01-001, 01-002 | 5 | M |
| 6 | **Fixed 2026-10-29.** Bloom: SE's 6-level pyramid at 0.5, threshold 0 (tune together with 5, it carries the star glare) | 09-001 | 4 | M |
| 7 | **Fixed 2026-10-29.** Clouds lit through the atmosphere (reddened sunlight, sky light, haze over them, no lit night side) | 05-001 | 5 | M |
| 8 | **Fixed 2026-10-29.** Seeing through the sky: per-channel transmittance, so the Moon shows by day and objects redden near the horizon | 05-006 | 4 | M |
| 9 | **Fixed 2026-10-29.** Surface tint and airless shading: catalog ModulateColor (Mars, Venus, Titan) and Hapke with opposition surge (full Moon) | 04-003, 04-001 | 4 | S |
| 10 | **Fixed 2026-10-29.** Oblate bodies: Saturn, Jupiter, Uranus, Neptune flattened, triaxial bodies | 04-002 | 4 | M |
| 11 | **Fixed 2026-10-29.** Moon's rotation (Moon-DE) and TidalLocked: the Moon keeps its near side to Earth | 07-008 | 4 | M |
| 12 | **Fixed 2026-10-29.** Catalog stars without catalog planets get SE's generated system (Vega, Sirius and every HIPPARCOS star are empty now) | 06-001 | 4 | M |
| 13 | **Fixed 2026-10-29.** Go to and Center timing: distance-dependent flights up to 20 s, double-G fast Go to, 1.5 s Center | 07-004 | 4 | M |
| 14 | **Fixed 2026-10-29.** Selection info as SE's upper-left text table with its fields, and Find object as a dialog that only selects | 08-005, 08-015, 08-006 | 5 | M |
| 15 | **Fixed 2026-10-29.** SE's four toolbars (main, filters, navigation, camera) | 08-003 | 5 | L |

After the top 15: cloud layers and atmosphere height (05-002, 05-004) and the interface skin (08-002) are **fixed
2026-10-29**, then the overlays (skin colours, SE's orbit lines, label modes and filters, keys and splash messages:
08-011, 08-009, 08-010, 08-016, 08-012), ring lighting (05-007) and galaxy front textures (03-001). The main menu
(08-001), the universe map (08-007) and context music (10-001) are left out for now.


## Summary

242 gaps reported by the auditors, 241 kept after checking (61 of them amended), 1 refuted. 81 have been fixed since (listed at the end); the tables count the 160 still open.

| Visibility | 5 | 4 | 3 | 2 | 1 |
|---|---|---|---|---|---|
| Gaps | 1 | 2 | 70 | 67 | 20 |

| Effort | S | M | L |
|---|---|---|---|
| Gaps | 54 | 76 | 30 |

| Area | Kept |
|---|---|
| 01 Stars | 17 |
| 02 Star clusters | 12 |
| 03 Galaxies and nebulae | 14 |
| 04 Planet surfaces | 11 |
| 05 Atmospheres, rings, comets | 14 |
| 06 Procedural generation | 14 |
| 07 Camera and time | 6 |
| 08 Interface | 26 |
| 09 Graphics and settings | 17 |
| 10 Scripts, audio, spacecraft | 29 |

Visibility: 5 = obvious in the first minutes with default settings, 4 = common activities, 3 = a specific common feature, 2 = niche, 1 = obscure. Effort: S about a day, M a few days, L a week or more or further reverse engineering. Status: missing / partial / differs. IDs are `<area>-<n>`; the raw auditor and checker lines are in `.audit/gaps/` and `.audit/checks/`.


## Visibility 5: obvious at once

### 08-001 · Interface

Main menu: SE opens in its main menu (3D scene background from menu/Scene1.se when Use3Dmenu, else Menu.pak skybox menu0-3; logo, version line; buttons Continue / Planetarium / Flight simulator / Settings / Tools / Credits / Exit with sub-menus, tutorials list, credits.txt, faq.txt; Esc returns to it); port starts straight in the simulation with no menu (Scene1.se runs only from its Scripts window)

- Status: missing, effort **L**, auditor confidence high
- SE evidence: Scripts.pak menu/MainMenu.cfg; Menu.pak menu0-3*.jpg; main-def.cfg Use3Dmenu; rtti seMainMenu
- Port: none
- Check: Scripts.pak has menu/MainMenu.cfg, credits.txt, faq.txt, Scene1.se, Menu.pak has menu0-3 skyboxes, main-def.cfg:538 Use3Dmenu true, rtti seMainMenu, strings "Planetarium"/"Flight simulator"/"Continue"; the port has no menu state anywhere (no Escape handler outside ship.rs:487, no menu strings in crates/), it starts in the simulation and only plays scripts/menu/Scene1.se via --script or the Scripts window (scripting.rs:300 just hides the HUD for it).


## Visibility 4: common activities

### 08-007 · Interface

Universe map (F1/M) missing: map centred on the selection or current position with a zoom slider from moons to the cosmic web, grid on the galactic plane with drop lines to objects (star_map_line), star marks coloured by temperature (pink for black holes/planemos), galaxy/cluster/nebula marks, clip-sphere fade, orbits and asteroids/comets as dots, exploration mode (overlays hidden, Density slider), MapShowProcedural/Grid/Lines, MapLabelsMode, MapPointOverbright, map renders of galaxies/nebulae (galaxy_model_map, nebula_model_map), time flow, Center/Close buttons

- Status: missing, effort **L**, auditor confidence high
- SE evidence: main-def.cfg UniverseMap 651-664; star_map_mark/line/point.glsl; galaxy_map_mark/line/point.glsl; map_sphere.glsl; rtti seMapMode; tut-05-univ-map strings
- Port: none
- Check: main-def.cfg UniverseMap 651-661 (MapExploreMode, MapShowProcedural/Grid/Lines, MapLabelsMode, MapDensity, MapPointOverbright), shaders star_map_mark/line/point, galaxy_map_*, galaxy_model_map, nebula_model_map, map_sphere exist (star_map_mark.glsl:42 magenta for marks without colour), tutorial "Open the map by pressing [F1] or the [M] key", rtti seMapMode; port has no map mode at all (F1 = help hud.rs:66, M = music music.rs:183, galaxies.rs map_mode is only a coordinate mapping)

### 10-001 · Scripts, audio, spacecraft

Context music: in SE's default context mode (MusicOrderMode 2) the track is picked from music/context.cfg for where you are: Object entries first (Earth, Moon, Mars, Saturn, Europa, Pluto, Betelgeuse, NGC 501, M 42, Sirius), then the most specific Class filter for the selected/nearest object (planet class + temperature/volatiles/life subclasses, star types, BlackHole, NeutronStar, Galaxy, Nebula, Cluster, Asteroid, Comet) falling back to more general ones, plus the mode classes MainMenu, Credits, MapMode, ChartMode; it switches when the context changes, but not before MinTrackPlayingTime 20 s. Port shuffles the union of all Class tracks with no regard to location, never uses Object entries or the mode classes

- Status: missing, effort **M**, auditor confidence high
- SE evidence: data/music/context.cfg header and 66 entries; main-def.cfg Sound MusicOrderMode, MinTrackPlayingTime, MixDurationContext; exe MusicClassSelector, MusicObjectSelector (FUN_140397fb0)
- Port: crates/se_engine/src/music.rs:113-139
- Check: context.cfg has the 4 mode classes, 54 Class and 12 Object entries (incl. Sirius A/B) with the most-specific-first filter rules in its header, main-def.cfg Sound MusicOrderMode 2 / MinTrackPlayingTime 20, and FUN_140397fb0 (seMusicPlayer::LoadConfig) references MusicClassSelector/MusicObjectSelector; port music.rs track_pool() just unions every non-MainMenu/Credits Class file list into one shuffled pool, ignores Object entries and never looks at the camera's location or mode


## Visibility 3: specific features

### 01-008 · Stars

oblate stars and gravity darkening: catalog Oblateness/DimensionsSol on stars (Altair 0.197, Vega 0.16, Achernar DimensionsSol (11.4 7.3 11.4)) and SE's gravity darkening (temperature * gravity^0.25 by latitude); port draws every star as a sphere with uniform brightness

- Status: missing, effort **M**, auditor confidence high
- SE evidence: Catalogs.pak planets/Stars.sc Oblateness, planets/Stars-bin.sc DimensionsSol; planet.glsl:1294-1297
- Port: crates/se_engine/src/system.rs:254 (MeterScale splat radius)
- Check: Catalog Oblateness 0.197/0.1618 (planets/Stars.sc:34,48) and DimensionsSol (11.4, 7.3, 11.4) (Stars-bin.sc:10345) exist, and planet.glsl:1294-1297 applies gravity darkening (surfTemp *= gravity^0.25 by sin^2 latitude); the port spawns every star with MeterScale(DVec3::splat(radius_m)) (system.rs:254), and se_engine never reads oblateness, so stars are uniform spheres

### 01-009 · Stars

brown dwarfs: SE draws them with tg_browndwarf_glow (gas-giant cloud bands mixed with granulation, temperature from band brightness); port draws L/T/Y dwarfs as a star photosphere with granulation plus a large red corona (Luhman 16 A shows a sun-like disc in a red haze)

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: tg_browndwarf_glow.glsl, tg_browndwarf_height.glsl
- Port: crates/se_engine/src/system.rs:259-283, crates/se_engine/src/corona.rs:94-118
- Check: SE has dedicated tg_browndwarf_{color,glow,height}.glsl (the glow mixes gas-giant cloud colour/height with GlowMapSun granulation, lines 10-16); the port has no brown-dwarf branch in system.rs or star_surface.wesl (every non-black-hole star gets StarSurfaceMaterial, system.rs:259-283), and corona.rs:94-118 gives every StarSphere the same 7-radius corona

### 01-010 · Stars

coronae: SE's corona is procedural fbm rays (RayDensity, RayCurvature, SinFreq) animated over time, or a texture corona blended between two textures (corona_plane_tex, StarCoronaTextures 10), sized and tuned per star by the catalog Corona block (Radius, Period, Brightness, RayDensity, RayCurv; NoCorona); port draws a fixed 7-radius shell with static noise streamers and ignores the catalog Corona block (parsed in body_params.rs, never used) and NoCorona

- Status: differs, effort **M**, auditor confidence high
- SE evidence: corona_plane_proc.glsl:61-92; corona_plane_tex.glsl:31-46; main-def.cfg StarCoronaTextures; Catalogs.pak planets/Stars-bin.sc Corona{Radius Period Brightness RayDensity RayCurv}
- Port: crates/se_engine/src/corona.rs:20,94-118, crates/se_engine/assets/shaders/corona.wesl
- Check: corona_plane_proc.glsl:61-92 builds fbm rays driven by Shape/Bright uniforms and corona_plane_tex.glsl:33 blends two textures (StarCoronaTextures 10, main-def.cfg:114); Stars-bin.sc:10879 has Corona{Radius Period Brightness RayDensity RayCurv} and NoCorona is an exe string; the port's CoronaParams (body_params.rs:288-310) is never read by se_engine, and corona.rs uses fixed EXTENT 7 with static vnoise streamers (corona.wesl:73-77)

### 01-011 · Stars

lens flare occlusion: SE scales each flare by the real frame/bloom brightness at the sun's screen position (switching between frame and bloom by sun size, LensFlareSmallMethodRadius 5 / LargeMethodRadius 8), so terrain, clouds, rings, ships and atmospheric extinction dim it; port computes occlusion analytically from body spheres only (a sun setting behind a mountain or clouds keeps its full flare)

- Status: partial, effort **M**, auditor confidence high
- SE evidence: lens_flare.glsl:137-161; main-def.cfg LensFlareSmallMethodRadius, LensFlareLargeMethodRadius
- Port: crates/se_engine/src/lensflare.rs:392-459
- Check: lens_flare.glsl:137-161 samples FrameTex/BloomTex at the sun's screen position (mixing between them by sun size) and scales the flare by that colour, so anything drawn over the sun dims or tints it; the port's lensflare.rs:391-459 only tests body spheres (plus a horizon plane and sea when inside one) analytically, and lens_flare.wesl samples nothing from the frame, so terrain, clouds, rings, ships and atmospheric extinction or reddening do not affect it

### 01-013 · Stars

black hole lensing background: SE samples the frame for rays landing on screen and blends to an environment cubemap of the whole sky (EnvMap, SkyRot) for rays leaving the screen or behind the camera, so the deep interior of the Einstein ring shows the true rear sky, and adds a fake gravitational blueshift/flux boost of the sky near the horizon; port uses only the finished screen image, mirror-repeated and faded to black off-screen, with no sky blueshift

- Status: differs, effort **M**, auditor confidence high, amended by the checker
- SE evidence: einstein.glsl:17-19 (samplerCube EnvMap)
- Port: crates/se_engine/assets/shaders/blackhole_lens.wesl
- Check: einstein.glsl:300-358 (ENV_MAP, BlackHoleQuality>=2, default 3) mixes the Frame texture with the EnvMap cubemap by a screen-edge/deflection blend, so SE also lenses on-screen content from the frame (no depth test outside WARP, so foreground objects are frame-lensed in SE too) and only off-screen/behind-camera/near-horizon rays use the true sky, plus SkyBlueShift (einstein.glsl:175-190) near the horizon; port blackhole_lens.wesl:217-240 mirrors the screen and fades to black off-screen and has no blueshift (docs/blackholes.md Limitations).

### 01-012 · Stars

accretion jets: SE draws relativistic jets from accretion disks (accretion_jet.glsl rotating particle jets, plus a volumetric jet in the GR black-hole shader) from catalog keys JetLength(Km), JetStartRadius, JetEndRadius, JetVelocity, JetStartTemp, JetEndTemp, JetDistortion, JetThickness, JetDensity, JetBrightness (hdrAccretionJetBright, ShowAccrJets); the port parses the keys but draws no jets (SS 433, Sgr A* show none)

- Status: missing, effort **L**, auditor confidence high, amended by the checker
- SE evidence: accretion_jet.glsl; gr_intersection.glh:37-48,206; exe Jet* keys
- Port: none (docs/blackholes.md "no jets")
- Check: Jets are real (accretion_jet.glsl rotating particle jets; volumetric cone jet in gr_intersection.glh:203-209; Jet* keys in planets/BlackHoles.sc e.g. SS 433 lines 49-55, SagittariusA.sc; ShowAccrJets main-def.cfg:684, hdrAccretionJetBright :266), and the port has none (docs/blackholes.md:121 "no jets", Jet* parsed in body_params.rs:373-408 but unused); but the pulsar part is unsupported, since planets/NeutronStars.sc has no Jet keys, and "precess" is not in the jet shaders

### 01-014 · Stars

accretion disk look: SE ray-marches a volumetric disk with thickness and detail (InnerThickness/OuterThickness, DetailScaleR/V, OctaveDistortion, OctaveScale, TwistMagn, DiskOuterSpin, DiskTempContrast, ShadowContrast/ShadowLength, LightingBright) and Planck emission from Temperature (hdr/realAccretionDiskBright); port's disk is infinitely thin with an artistic colour mapping of Temperature/Brightness and ignores those keys

- Status: differs, effort **L**, auditor confidence high
- SE evidence: gr_intersection.glh:28-61,200,305-309; einstein.glsl:41-46; exe disk keys
- Port: crates/se_engine/assets/shaders/blackhole_lens.wesl, crates/se_engine/src/blackholes.rs:173
- Check: gr_intersection.glh:20-36,300-309 define DiskParams1-5 (thickness inner/outer, detail scale R/V, octave distortion/scale, twist, outer spin, temp contrast) and PlanckFunction emission with Doppler/grav shift; port accretion_disk.wesl:103-133 is a single equatorial-plane crossing with temp_k = 4000*(T/1e4)^0.3 and Brightness^0.1 compression (blackholes.rs:173-200), docs/blackholes.md admits thickness keys ignored.

### 02-008 · Star clusters

Radial profile and CenPow: SE uses the CenPow key (158 catalog clusters, procedural N(1.5,0.3) in [0.5,2.5]) as concentration: exponential profile exp(-c r/R) with c = clamp(7.3*sqrt(CenPow),5,30) for globular/nucleus, Gaussian blob with k = 1/CenPow for open; port ignores CenPow (not parsed; proc_galsys draws it and discards it) and uses one Plummer profile with a = 0.5R/1.305

- Status: missing, effort **S**, auditor confidence high
- SE evidence: FUN_14053a760, FUN_140441860, FUN_14047fcb0 key CenPow
- Port: crates/se_engine/src/clusters.rs:328-343,387-388, crates/se_core/src/proc_galsys.rs:495
- Check: FUN_14047fcb0:470 reads CenPow (158 occurrences across Globular/Open/Extragalactic/SpaceEngine.sc), and FUN_14053a760 uses it as k = 1/CenPow in FUN_140441860 for open clusters and as c = clamp(sqrt(CenPow)*7.3, 5, 30) (consts 0x1406d5410/5318/5680) in an exp(-c r) acceptance test for the others; the port has no CenPow anywhere (ClusterRecord has no field), proc_galsys.rs:495 discards the draw, and build_cluster uses a fixed Plummer a = 0.5R/1.305.

### 02-009 · Star clusters

Central black holes: SE makes member 0 of the root node a black hole in every globular cluster older than 10 Myr (mass rand*sqrt(age/13)*1e4, clamped rand*100..rand*1e5 Msun) and in every galaxy-nucleus (Kern) cluster (mass rand*(galaxy[+0x20]/13700)^3*1e6, clamped rand*1e5..rand*1e10), unless a catalog star sits at the centre (FUN_140471df0); open clusters get none; the port has no cluster black holes and builds Kern clusters as Globular

- Status: missing, effort **S**, auditor confidence high, amended by the checker
- SE evidence: FUN_14053a760 (root-node branch, class 0x5e80, radius M*4.245e-6 Rsun), FUN_140471df0
- Port: crates/se_engine/src/clusters.rs:395-415, crates/se_engine/src/procsystems.rs:576
- Check: In FUN_14053a760 the black-hole branch is only in the non-Open path (iVar18 == 3 takes the Gaussian loop with no black hole), so open clusters never get one; the rest checks out (0x5e80, lum 1000, radius M*4.245e-6, age > 0.01 via 0x1406d3f68, rand*sqrt(age/13)*1e4 clamped [rand*100, rand*1e5], Kern rand*(g[+0x20]/13700)^3*1e6 clamped [rand*1e5, rand*1e10]), and the port has none, with procsystems.rs:576 mapping Kern to "Globular".

### 02-011 · Star clusters

Member and procedural star systems: SE member stars carry the cluster's age (and their own metallicity) into planet generation; the port's proc_system_input (system.rs:406-407) generates every promoted procedural star's system, cluster members included, with age 4 Gyr and [Fe/H] 0, because MemberStar/ProcStarInfo hold no age or metallicity

- Status: differs, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: FUN_1401d6c60 / FUN_1401d63b0 (age = cluster age)
- Port: crates/se_engine/src/system.rs:406-407, crates/se_engine/src/clusters.rs:240-279
- Check: Both sides check out (FUN_14053a760 copies cluster age +0x3c into each member before FUN_1401d6c60/FUN_1401d63b0; port system.rs:406-407 hard-codes age_gyr 4.0, feh 0.0, which procgen/mod.rs:408-409 then use for planets), but the hard-coding is in proc_system_input for every promoted procedural star, not only cluster members, and MemberStar/ProcStarInfo carry no age to pass on; promoting the single nearest star is the port's general mechanism, not the gap.

### 02-012 · Star clusters

Cluster info: SE's info panel shows the cluster type (globular#star_cluster_type / open), magnitudes, luminosity, age and star count ("Number of stars", "Fly inside to count stars"); port's Selection panel shows only kind, distance, RA/Dec and radius

- Status: partial, effort **S**, auditor confidence medium
- SE evidence: eng-gui.cfg globular#star_cluster_type, FUN_14038a450 L"Number of stars" L"Fly inside to count stars"
- Port: crates/se_engine/src/hud.rs:107,313-317 (no cluster branch)
- Check: SE loads globular/open#star_cluster_type (eng-gui.cfg:124-125) and "Number of stars"/"Fly inside to count stars" in FUN_14038a450:922-923,1347-1350; the port's hud.rs shows "<kind> cluster", distance, RA/Dec and radius, and its per-type block (hud.rs ~250-320) has no Cluster arm, so there is no magnitude, luminosity, age or star count (it does show the type, as the gap says).

### 02-013 · Star clusters

Cluster markers and labels: SE draws green-diamond markers for clusters (ShowMarkersClusters true by default) and optional labels (ShowLabelsClusters), alpha = clamp(brightness - ClusterBrightMin) * clamp((radiusPx - 2)/3); port has no cluster marker kind or labels

- Status: missing, effort **S**, auditor confidence medium
- SE evidence: main-def.cfg ShowMarkersClusters/ShowLabelsClusters, FUN_140488a20 case 2, FUN_14049a460, locale "green diamonds highlight star clusters"
- Port: crates/se_engine/src/overlay.rs:61-67
- Check: SE: main-def.cfg:748 ShowMarkersClusters true / :733 ShowLabelsClusters false, locale (cat/fra/ger-gui.cfg) "green diamonds highlight star clusters", FUN_140488a20 case 2 computes alpha = clamp(lum*K/d^2 - ClusterBrightMin(DAT_14075f804)) * clamp((radiusPx - 2)/DAT_1406d5008); port overlay.rs:61-67 MarkerKind has only Planet/Moon/Minor/Star/SystemStar, clusters appear only as pick discs (overlay.rs:486) and in the HUD kind text, no marker/label code anywhere in clusters.rs/procsystems.rs

### 02-006 · Star clusters

Open-cluster members: SE draws them by luminosity budget (stars from FUN_1401d6c60: Mv N(3,4) evolved at the cluster age into main-sequence stars, giants or white dwarfs, metallicity N(-0.125,0.07), each <= 0.5 L_cluster, until the sum reaches L_cluster, <=16384); port samples a Salpeter mass function truncated at the turn-off with no remnants and count = L / mean luminosity

- Status: differs, effort **M**, auditor confidence high
- SE evidence: FUN_14053a760 (type 3 branch), FUN_1401d6c60
- Port: crates/se_engine/src/clusters.rs:291-326,371-376
- Check: FUN_14053a760:144-185 open branch loops FUN_140441860 positions (k = 1/CenPow) and FUN_1401d6c60 stars, accepts a star only if its lum <= L_cluster*fVar33, stops when the summed lum reaches L_cluster or 16384 (0x3fff) and writes the sum back to the record; port clusters.rs:291-326,371-376 samples Salpeter masses below a turn-off with a fixed 0.2% giant fraction (only for age > 1 Gyr), no white dwarfs, count = L / mean_lum, Plummer positions ignoring CenPow

### 02-007 · Star clusters

Globular/nucleus member physics: SE generates each member with the full procedural star generator inside the level's magnitude band (per-class age and metallicity, evolved giants, white dwarfs, neutron stars, black holes, and a 45 % globular / 60 % nucleus share of special objects at the leaf level); port has main-sequence stars plus 0.2 % generic giants only, classes V/III

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: FUN_1401d63b0, FUN_140543c70 leaf factors 0x140807fdc/0x140807f5c
- Port: crates/se_engine/src/clusters.rs:298-312,408-414
- Check: FUN_14053a760 calls FUN_1401d63b0(class 4 Kern / 3 globular, table[level] Mbright,Mfaint), which runs the evolution path (FUN_1401dd6b0, FUN_1401d5fe0) and diverts 0.6 (0x14075f828, Kern) / 0.45 (0x14075f82c, globular) of draws to class-0x6800 special objects, with FUN_140543c70 scaling leaf counts by 0x140807f5c/0x140807fdc; the port's clusters.rs sample_star is Salpeter main sequence plus giant_frac 0.002 (age > 1 Gyr only) and member_info labels only V/III.

### 02-010 · Star clusters

Member star names: SE names them RSC <galaxy>-<cluster>-<level>-<node>-<index> (8-number form in procedural galaxies) and resolves those names in search, goto and saved places; port names them "<cluster name> #k" and cannot resolve RSC names

- Status: differs, effort **M**, auditor confidence high
- SE evidence: FUN_1401c0180 L"RSC %i-%i-%i-%I64d-%i", parser FUN_1401c2480
- Port: crates/se_engine/src/clusters.rs:89-99
- Check: FUN_1401c0180 formats L"RSC %i-%i-%i-%I64d-%i" and the 8-number form, and the %u scanf variants in re/strings_utf16.txt:747-748 show the parse-back; the port's clusters.rs:94 uses format!("{cluster} #{k}") and nothing in crates/ handles "RSC" (only RS field-star names exist), and SE-compatible names need SE's member octree level/node ids.

### 03-002 · Galaxies and nebulae

Dust of galaxy and nebula sprite models: SE absorption sprites output (-colour*emBright*fade, absBright*fade) with blend ONE, ONE_MINUS_SRC_ALPHA (darken by absBright x the model's Transparency and subtract the dust colour); port instead uses its own Beer-Lambert multiply exp(-tau*desaturated tint*6) with ad-hoc global strengths (0.35 galaxies, 0.06 nebulae "deliberately weak"), so dust strength and colour differ: nebula dust (Horsehead, Barnard) is near-invisible while the edge-on Milky Way lane renders as a dark brown band; main-def.cfg ColorAbsorption is not read by the port

- Status: differs, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: galaxy_model.glsl:64-69; nebula_model.glsl (same, texture .a); FUN_14029bdf0 glBlendFunc(1,0x303); main-def.cfg ColorAbsorption
- Port: crates/se_engine/assets/shaders/galaxy_model.wesl:113-118; galaxies.rs:229 dust_strength; docs/formats/nebula.md section 7
- Check: Code difference confirmed: galaxy_model.glsl:64-69 and nebula_model.glsl:60-65,77 output (-rgb*emBright*fade, absBright*fade) drawn with glBlendFunc(1,0x303) (FUN_14029bdf0:194), while galaxy_model.wesl:113-118 uses exp(-tau*desaturated tint*6) multiply with global strengths 0.35/0.06 (galaxies.rs:227, nebulae.rs:66, nebula.md:176 "deliberately weak"); but the M42 "near-black bands" claim is not shown by the auditor's runs (r1_orion_crop shows no such band; r1_mw does show a dark brown edge-on lane), and ColorAbsorption feeds DAT_14082c21e/21f read in FUN_1404877f0 (seCustomModel::Draw), not the galaxy sprite shader, so its role here is unproven

### 03-007 · Galaxies and nebulae

Nebula NoModel key ignored: SE gives catalog nebulae with `NoModel true` no model (M 43 and Sh2-279 Running Man, which are drawn by the Orion Nebula model); port gives them a pooled ray-marched model, so a stray striped orange sphere sits on the Running Man Nebula next to M 42

- Status: missing, effort **S**, auditor confidence high
- SE evidence: FUN_140172400 ("NoModel" -> model index 0xfffe); Catalogs.pak nebulae/SpriteAstro.sc M 43, Sh2-279 NoModel true
- Port: crates/se_engine/src/nebulae.rs:327-329 (pooled raymarch for every nebula without a named model)
- Check: FUN_140172400:462-464 reads "NoModel" and sets model index 0xfffe, and SpriteAstro.sc gives M 43 and Sh2-279 `NoModel true` ("implemented as a part of M 42 model"); the port's NebulaRecord (se_core/src/catalog.rs:131) has no such field, nothing in crates/ reads NoModel, and nebulae.rs:327-329 falls back to custom.pooled() for every nebula without a named model (run .audit/work/03/r3_sh2279.png).

### 03-008 · Galaxies and nebulae

Nebula sprite atlas: SE nebula sprite models sample their own atlas sprites_neb.png (Common.pak, 1024x1024 single-channel greyscale, 4x4 tiles of filamentary cloud shapes, used as coverage); port draws nebula sprite models with the galaxy atlas sprites_gal.png, so nebula texture detail differs

- Status: differs, effort **S**, auditor confidence high, amended by the checker
- SE evidence: nebula_model.glsl:76 (texture(...).a); FUN_140168bd0 "sprites_neb.*"; Common.pak sprites_neb.png
- Port: crates/se_engine/src/galaxies.rs:714 (only sprites_gal.png loaded, shared with nebulae.rs)
- Check: FUN_140168bd0:177-181 loads "sprites_neb." next to sprites_gal, and nebula_model.glsl:76 samples the texture as coverage; the port loads only textures/common/sprites_gal.png (galaxies.rs:714) and nebulae.rs reuses that SpriteAtlas; but Common.pak sprites_neb.png is a 1024^2 8-bit greyscale (PNG colour type 0, mode L) 4x4 tile set of filamentary clouds, not an alpha-channel image.

### 03-010 · Galaxies and nebulae

Galaxy redshift: SE multiplies galaxy sprites and the front plane by Reddening = (1-x, 1-2x, 1-4x), x = distance / 4.227e9 pc, which reddens and dims distant galaxies (blue gone near 1 Gpc, green near 2 Gpc); port tints only far sprites with a 5500 K Planck spectrum shifted by z at constant luminance (about 10% redder at 1 Gpc), and 3D models are not tinted

- Status: differs, effort **S**, auditor confidence medium
- SE evidence: FUN_140299fe0 (param_1[4..6] = 1 - d/4.227e9 * (1,2,4); constants 0x14075eb8c, 0x1406d523c); galaxy_model.glsl:72; galaxy_plane.glsl:57
- Port: crates/se_engine/assets/shaders/galaxy_far.wesl:117-130 (redshift_tint); galaxy_model.wesl (none)
- Check: FUN_140299fe0:251-254 computes param_1[4..6] = 1 - d/DAT_14075eb8c * (1, 2, DAT_1406d523c) with bytes 0x4f7bf2e7 = 4.227e9 and 0x40800000 = 4.0, and galaxy_model.glsl:72 / galaxy_plane.glsl:57 multiply colour by Reddening.rgb (dimming, not luminance-preserving); the port only has redshift_tint() in galaxy_far.wesl:120-127 (5500 K Planck shift normalised to constant luminance) and galaxy_model.wesl has no reddening.

### 03-003 · Galaxies and nebulae

Galaxy sprite-model shading and brightness: SE fades a sprite when the eye is within 1.5 sprite radii (clamp(dist/spriteSize-0.5)) and scales models by hdr/realGalaxySpriteBright x ModelBright x pst Brightness (BrightZoom); port fades sprites by on-screen size (250-875 px) with edge-faded quads, enlarges sub-pixel sprites to a minimum radius, dims all bulge sprites to 0.3 (BULGE_GAIN) and calibrates model brightness to its analytic far sprite, so the inside-galaxy look (local clumps, bulge glare, overall model brightness) differs

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: galaxy_model.glsl:39-57; FUN_14029bdf0 BrightZoom; main-def.cfg hdrGalaxySpriteBright 0.03, realGalaxySpriteBright 0.01
- Port: crates/se_engine/assets/shaders/galaxy_model.wesl:61-69,108; crates/se_engine/src/galaxies.rs:960 (BULGE_GAIN), 1193 (calibration)
- Check: galaxy_model.glsl:39,56 fades by clamp(dist/spriteSize-0.5) (gone inside 0.5 sprite radii) vs galaxy_model.wesl:61-69 screen-size fade between big_px 250 and 875 px plus MODEL_MIN_PX enlargement and edge-faded quads; BULGE_GAIN 0.3 (galaxies.rs:960) and far-sprite calibration (galaxies.rs:1187-1195) have no SE counterpart; SE emBright comes from FUN_140299fe0 fade x galaxy+0xa4 and renderer factors (FUN_14029bdf0:152-154), the hdr/realGalaxySpriteBright cvars being read in FUN_1404c4ae0, so the brightness chain differs even if the exact cvar wiring is indirect

### 03-004 · Galaxies and nebulae

Far galaxies and model LOD: SE draws unresolved galaxies as star-like point sprites (StarPoint/StarSprite programs built in the galaxy init) and fades the real sprite model in between 2 and 5 px radius for every such galaxy (model cache CacheSizeGalModels 64); port draws every galaxy as an analytic-profile elliptical billboard (de Vaucouleurs / disc+bulge / irregular) until 6-16 px and keeps at most 6 models (MAX_SLOTS 8), so in groups and clusters most resolved galaxies stay smooth blobs without structure

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: FUN_140168bd0 (StarPointUniforms/StarSpriteUniforms in seGalaxyCatalog init); FUN_140299fe0 fade_in=clamp((px-2)/3); main-def.cfg CacheSizeGalModels 64
- Port: crates/se_engine/src/galaxies.rs:225-231 (build_px 4, max_models 6), 993 (model_fade 6..16 px); galaxy_far.wesl
- Check: FUN_140168bd0 builds star_point/star_sprite(+blur) programs with GALAXY define, whose star_sprite.glsl:95-110,202-207 draws a round 1/r^2 flare that fades as RadiusPix grows, and FUN_140299fe0 fades the model in with clamp((px-c)/d) at a few px for every galaxy (cache CacheSizeGalModels 64); port draws analytic elliptical billboards (galaxy_far.wesl) and fades models only between 1.5x and 4x build_px 4 = 6..16 px (galaxies.rs:993-999) with max_models 6, MAX_SLOTS 8 (galaxies.rs:54,230)

### 03-005 · Galaxies and nebulae

Elliptical galaxies: SE draws every resolved E galaxy with galaxy_bulge.glsl from the pre-integrated density table galaxy_bulge.tif (PrecomputedDensityFunc), sized GalaxyEllipticalScale 2.4 x the catalog radius, coloured GalaxyEllipticalColor (1 0.9 0.7) x GalaxyEllipticalBright, with Reinhard + GalaxyEllipticalGamma 0.7 colour correction; port ray-marches its own Prugniel-Simien fit inside 1.03 R, tinted by its Hubble-type colour, no gamma/HDR correction, for the nearest 4 ellipticals only

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: galaxy_bulge.glsl:35-52; FUN_140168bd0 (galaxy_bulge.tif, LoadBulgeLookupTexture); main-def.cfg GalaxyGenerator 1048-1056
- Port: crates/se_engine/src/ellipticals.rs:28-30 (MAX_VOLUMES 4, BOUND 1.03), 204 (type_color); crates/se_engine/assets/shaders/galaxy_elliptical.wesl
- Check: galaxy_bulge.glsl:35-52 reads the pre-integrated PrecomputedDensityFunc table (galaxy_bulge.tif loaded in FUN_140168bd0:96) and applies Color*density*12 then ColorCorrentionHDR (Reinhard, hsl.glh:62-65) *10; main-def.cfg 1051-1055 GalaxyElliptical Scale/Color/Bright/Gamma exist; port ellipticals.rs ray-marches its own Prugniel-Simien profile (MAX_VOLUMES 4, BOUND 1.03), coloured by type_color (line 204), and reads none of the GalaxyElliptical* keys nor any gamma/Reinhard step

### 03-006 · Galaxies and nebulae

Ray-marched nebula shader is a reimplementation that differs from the decrypted nebula_raymarch.glsl: SE gas emission = e*colour*coef_1*exp(-e^coef_2*coef_3) with e built from the SDF distance by Emission_gas_N_direction/coef_0 (port: coef_3*exp(-max(r-coef_1,0)/coef_2), r = distance from coef_0); SE erodes the bounding shape and the star cavity with noise (abs(1-1.2*noise)) where the port uses smooth fades; SE applies a smootherstep curve sum^3(6s^2-15s+10) and writes a separate transmittance; SE dithers density and step length (Dithering_func_1/2, hash and DitherNoiseTex) where the port only offsets the first sample per pixel with fixed steps; so ray-marched nebula shapes, edges and colours differ from SE

- Status: differs, effort **M**, auditor confidence high, amended by the checker
- SE evidence: nebula_raymarch.glsl (main, NebulaMap, Dust_SDF_Volume); nebula_raymarch_uniforms.glh; raymarch_common.glh DitherNoiseTex
- Port: crates/se_engine/assets/shaders/galaxy_nebula_raymarch.wesl; docs/formats/nebula_raymarch.md section 4; run .audit/work/03/r3_m43.png
- Check: Emission, erosion and tone-curve differences confirmed (nebula_raymarch.glsl:26-36 abs(1-1.2*noise) erosion of bound and star cavity, 65 e*colour*coef_1*exp(-e^coef_2*coef_3), 75 smootherstep curve and separate OutTransm vs galaxy_nebula_raymarch.wesl:237-250 smooth edge/bubble fades, 287 coef_3*exp(-max(r-dir.w,0)/c0.w), 387 plain output), but "port has no dithering" is wrong: the port jitters the first sample per pixel (wesl:333-334,354); SE instead dithers density and step length (Dithering_func_1/2 with hash and DitherNoiseTex), and the striping on the Running Man in r3_m43.png is not shown to be step banding

### 03-009 · Galaxies and nebulae

Nebula sprite-model brightness and residency: port calibrates each model to the AbsMagn-based far sprite, so compact planetaries saturate to white close up (M 57 at 3e13 km) while faint ones (Abell 33, Manatee) are nearly invisible, keeps at most 4 resident models, and draws distant shell nebulae as a soft disc; SE scales models by hdrNebulaSpriteBright 0.06 / realNebulaSpriteBright 0.02 x model Brightness with NebulaBrightMin/GlowBrightMin cut-offs

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: main-def.cfg hdrNebulaSpriteBright, realNebulaSpriteBright, NebulaBrightMin, GlowBrightMin, CacheSizeNebModelsVRAM
- Port: crates/se_engine/src/nebulae.rs:617; docs/formats/nebula.md sections 6-7; run .audit/work/03/r1_m57.png
- Check: Port calibrates model scale to the AbsMagn far-sprite surface brightness (nebulae.rs:613-617), caps residency at 4 (nebulae.rs:68, max 8), and docs/formats/nebula.md section 7 itself lists the M57 white saturation, near-invisible Abell 33/Manatee and soft-disc shell sprites (confirmed in .audit/work/03/r1_m57.png, white-cored blob); SE keys hdrNebulaSpriteBright 0.06 / realNebulaSpriteBright 0.02 / NebulaBrightMin 0.03 / GlowBrightMin 0.006 exist (main-def.cfg 173,175,253,275) and are read by FUN_1400125e0/FUN_14027a6a0, none is read by the port; exact SE application of the factors was not traced, so medium confidence.

### 03-011 · Galaxies and nebulae

Intergalactic vision algorithm: SE sums the sky coverage of drawn galaxies (R^2/(d^2-R^2) per galaxy) and eases a brightness factor 1/(a + b*coverage) at AutoGalaxyMagnSpeed (1-exp(-dt*speed)); port sets the gain so the 3000th brightest galaxy looks magnitude 7 (cap 3000), blended in from 1 to 2 galaxy radii out, with a fixed 0.3 s smoothing and AutoGalaxyMagnSpeed not read

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: FUN_1404c51a0, FUN_140299fe0 (+0x35cc accumulator); main-def.cfg AutoGalaxyMagnitude, AutoGalaxyMagnSpeed 10
- Port: crates/se_engine/src/procgalaxies.rs:452-475; crates/se_engine/src/galaxies.rs:1045-1061
- Check: FUN_140299fe0:144-152 accumulates R^2/max(d^2-R^2, eps) into +0x35cc per drawn galaxy and FUN_1404c51a0:78-90 eases it with 1-exp(-dt*speed) and sets brightness 1/(a + b*coverage) clamped >= 1; the port instead derives gain from the 3000th-brightest galaxy mapped to mag 7 (procgalaxies.rs:452-475, clamp 1..3000), blends by smoothstep(1,2 radii) and smooths with a fixed k = 3*dt (galaxies.rs:1050-1058), and AutoGalaxyMagnSpeed appears nowhere in crates/.

### 03-012 · Galaxies and nebulae

Galaxy/nebula display controls: SE has galaxy and DSO magnitude limits (GalaxyMagnLimit, NebulaMagnLimit, GlowMagnLimit), galaxy brightness up/down, an Intergalactic vision toggle and catalog-object filters (ShowCatGalaxies, ShowCatNebulae: "Real galaxies only / All galaxies", "Real nebulae only / All nebulae", "Galaxies disabled/enabled"); port exposes only the ShowProc* checkboxes and env vars

- Status: missing, effort **M**, auditor confidence medium
- SE evidence: locale *-gui.cfg keys "Galaxies magnitude limit", "Increase galaxies brightness", "DSO magnitude limit", "Intergalactic vision", "Real galaxies only"; main-def.cfg 167-177, 666-671
- Port: crates/se_engine/src/procsystems.rs:169; galaxies.rs:241-266 (env only)
- Check: Locale keys "Galaxies magnitude limit", "DSO magnitude limit", "Increase/Decrease galaxies brightness", "Intergalactic vision", "Real galaxies only", "All galaxies", "Real nebulae only", "All nebulae", "Galaxies disabled" exist (fra-gui.cfg 1268-1466; eng-gui.cfg is only a 5 KB stub) and main-def.cfg 167-177, 666-670 hold GalaxyMagnLimit/NebulaMagnLimit/GlowMagnLimit/AutoGalaxyMagnitude/ShowCatGalaxies/ShowCatNebulae; the port's only deep-sky UI is the three ShowProc* checkboxes (procsystems.rs:169-171), hud.rs/graphics.rs sliders cover stars and flares only, and galaxy brightness/auto gain are env-only (galaxies.rs:241-266).

### 04-009 · Planet surfaces

Airless/icy moon colouring (tg_selena_color, tg_asteroid_color): bright rayed-crater ejecta (RayedCraterColorNoise; port has rayed craters only in height), leading/trailing hemisphere darkening (drivenDarkening generated in surface_gen.rs:1587 but never drawn), Europa-like freckles, ice-crack colour lines (CrackColorNoise; port cracks are height only), young-terrain albedo suppression, shield-volcano lava patches on selena bodies, asteroid flat-ground darkening (color *= 0.5 + slope) are missing; SE's thin 0.8 polar frost (latitude - latIceCaps + 0.3) differs from the port's ice biome/cold-cap approximation

- Status: missing, effort **M**, auditor confidence high, amended by the checker
- SE evidence: tg_selena_color.glsl:108-196; tg_common.glh:2344 RayedCraterColorNoise, 2682 CrackColorNoise; tg_asteroid_color.glsl:18-32
- Port: crates/se_engine/assets/shaders/planet.wesl:609-655; crates/se_core/src/surface_gen.rs:1587 (value unused in se_engine)
- Check: tg_selena_color.glsl:113-182 has young-terrain albedo suppression, CrackColorNoise, Europa freckles, drivenDarkening, RayedCraterColorNoise ejecta and 0.8 polar frost (plus shield-volcano lava at ~:82) and tg_asteroid_color.glsl:27 `color *= 0.5 + slope`; the port has none of these in colour (rayed craters/cracks/volcanoes only as height in tg_height.wesl:1091,1522; drivenDarkening() accessor never called; lava only on lava worlds), but it does port ColorVary albedo variation (surface_tex.rs:531) and has a polar ice biome (planet.wesl:551) and cold-world caps (planet.wesl:273), so frost is approximated rather than missing

### 04-010 · Planet surfaces

City lights and self-emission: SE switches night lights off by the actual light reaching the ground (sun plus sky light after eclipse and ring shadows, cutoff brightness), so lights come on in eclipses, and honours GlowMode "Permanent" (Io's Lights-SE glows in daylight) plus EnableSelfEmission, SelfEmissionBright, ToggleSelfEmission and GUI "Self-emission"/"Self-emission brightness"; the port uses a fixed sun-elevation smoothstep, treats every glow map as night lights, fades the lights out as the camera comes down (full only above a 600 m pixel footprint, gone below 30 m) and has no self-emission settings

- Status: differs, effort **M**, auditor confidence high
- SE evidence: planet.glsl:1153-1155,1319-1341; SolarSys.sc Io "GlowMode Permanent"; main-def.cfg EnableSelfEmission, SelfEmissionBright; def_generic_controller.cfg ToggleSelfEmission
- Port: crates/se_engine/assets/shaders/planet.wesl:1238-1247
- Check: planet.glsl:1153-1154 accumulates shadowed sun+sky light into cityLightingAccum and 1336-1338 cuts city lights by its luminance between SurfParams4.x*0.2 and SurfParams4.x, scaled by GlowColor.a (Night vs Permanent; SolarSys.sc:1354 Io "GlowMode Permanent", exe "Surface.GlowMode" errors), with main-def.cfg:104,112 EnableSelfEmission/SelfEmissionBright and ToggleSelfEmission; the port (planet.wesl:1238-1247) uses smoothstep(-0.15,0.05,dot(up,l)) ignoring eclipses, fades lights by smoothstep(30,600,footprint), never parses GlowMode (no match in crates/) and has no self-emission settings

### 04-013 · Planet surfaces

Detail-texture displacement ("3D stones"): SE raises close terrain vertices by the soil height map (DisplaceDetailTextures true, DisplaceDetailHeight 0.1 m per m), so stones and rubble have real relief near the ground; the port's soils only shade (normal maps), the mesh is never displaced

- Status: missing, effort **M**, auditor confidence high
- SE evidence: planet.glsl:441-467 DISP; main-def.cfg DisplaceDetailTextures, DisplaceDetailHeight
- Port: crates/se_engine/src/terrain.rs (mesh), crates/se_engine/assets/shaders/planet.wesl:846-900
- Check: SE's vertex stage adds `detHeight * SurfParams3.z` (DetDiffMap alpha / DetBumpMap) to the vertex height under DET+DISP (planet.glsl:441-466, inside _VERTEX_ 212-536; main-def.cfg:99-100; exe strings DISP, DisplaceDetailTextures), while the port's planet.wesl vertex() only geomorphs the CPU mesh and terrain.rs stops relief at FINEST_FEATURE_M 20 m, soils' rhao height is used only for shading/blending

### 04-014 · Planet surfaces

Faint square grid on smooth lit planet discs from orbit (Saturn at 250000 km, Kepler-10 b at 40000 km show a regular ~10 px checker pattern on the day side in runs); SE hides texel and mesh grids with jittered lookups and per-node normal maps, no grid shows

- Status: differs, effort **M**, auditor confidence low
- SE evidence: planet.glsl:577 PlanetUVShatter; terrain_noise.glh:89-91
- Port: crates/se_engine/assets/shaders/planet.wesl (terrain/aerial pass, cause not isolated)
- Check: The auditor's own runs (.audit/work/04/saturn.png, kep10_zoom.png) show a regular ~10 px checker on the lit discs of Saturn and Kepler-10 b, visible even at half size, and SE jitters its lookups (planet.glsl:576 PlanetUVShatter, terrain_noise.glh:89-91); cause in the port is still not isolated

### 05-015 · Atmospheres, rings, comets

Atmosphere at a distance: SE keeps the atmosphere at least AtmoMinWidthPix (2 px) thick on screen and limits its effect to AtmoDistanceLimit (30 radii), so distant planets keep a visible limb. The port shows the shell only once the body is over 0.5 px, at its true thickness, so the limb disappears below a pixel

- Status: missing, effort **S**, auditor confidence medium
- SE evidence: main-def.cfg AtmosphereRendering AtmoMinWidthPix AtmoDistanceLimit
- Port: crates/se_engine/src/system.rs:732
- Check: FUN_14044fe70 reads AtmoMinWidthPix (DAT_1419b2608) to widen the atmosphere height to at least tan(px*pixel angle)*distance, and AtmoDistanceLimit (DAT_1419b24d8) to clamp the eye distance to limit*radius for the atmosphere computation; the port has neither key, and update_sky_shells (system.rs:732) shows the true-thickness shell only once px_radius > 0.5.

### 05-017 · Atmospheres, rings, comets

Cloud shadows: the port darkens the ground under its cloud shell. SE 0.991's planet shader has no cloud-shadow term; its only shadows are from rings and eclipses

- Status: differs, effort **S**, auditor confidence medium
- SE evidence: planet.glsl:1057-1095 (Shadow = rings x eclipse only)
- Port: crates/se_engine/assets/shaders/planet.wesl:1228-1231
- Check: SE's planet.glsl has no cloud texture bound in the surface pass (samplers 0-15 contain no cloud map) and its Shadow term at 1057-1098 is only RingsShadow x eclipse, and no exe string or SE shader mentions a cloud shadow; the port multiplies the ground by 1 - 0.55*cloud in planet.wesl finish() (~1228-1231) and README says "cloud shadows"

### 05-009 · Atmospheres, rings, comets

Shadows on rings: SE casts the planet's shadow with a penumbra sized by the sun's angular radius and the planet's oblateness (EclipseValue, Oblateness), plus moon eclipse shadows on the 2D rings, the dust and the rocks (ECL/ECLDUST/ECLSTONES, e.g. SE's place "Titan's shadow on Saturn's rings"). The port tests the shadow ray against a sphere with a fixed smoothstep(0, 0.04) edge and draws no moon shadows on rings

- Status: partial, effort **M**, auditor confidence high
- SE evidence: rings.glsl:66-69; rings_raymarch.glsl:225,271-273,299-301; ger-gui.cfg "Titan's shadow on Saturn's rings"
- Port: crates/se_engine/assets/shaders/rings.wesl:73-83
- Check: rings.glsl:66-69 shades the planet shadow with EclipseValue(lightAngularRadius, casterAngularRadius, angle) on an oblateness-scaled light position and adds EclipseShadowAuto moon shadows under ECL (ger-gui.cfg:2466 cites the "Titan's shadow on Saturn's rings" place); the port's rings.wesl:73-83 does a sphere ray test with smoothstep(0,0.04) and has no eclipse import, while README says eclipses cover terrain, clouds and sky only

### 05-010 · Atmospheres, rings, comets

Ring shadow on planet, clouds and sky: SE's RingsShadow uses opacity 1-(1-density)^(1/cosPhi) with fwidth anti-aliased edges and fades out at equinox, and shadows the sky too (AtmoRingsShadow "Ring shadows on sky"); its "rings winter" (Clouds RingsWinter, 0 disables) fades clouds and the surface diffuse colour on the winter hemisphere by tan(sun elevation over the ring plane) x RingsWinter. The port draws a constant 0.95 x alpha hard-edged shadow on the ground only, nothing on sky or clouds, and rings_winter is parsed but unused

- Status: partial, effort **M**, auditor confidence high, amended by the checker
- SE evidence: rings_common.glh:278-309; sky.glsl:247-253; planet.glsl:513-533,949-955; main-def.cfg AtmoRingsShadow; SolarSys.sc RingsWinter
- Port: crates/se_engine/assets/shaders/planet.wesl:911-933; crates/se_engine/assets/shaders/sky.wesl
- Check: Shadow formula and missing sky shadow hold (rings_common.glh:278-309 1-(1-density)^(1/cosPhi) with fwidth smoothing and equinox fade; sky.glsl:247-253 RingsShadow; port planet.wesl:911-933 constant a*0.95, nothing in sky.wesl or clouds.wesl), but SE's rings winter (planet.glsl:513-533, 949-955) is not a removal of clouds under the shadow: it fades the clouds and the surface diffuse colour on the hemisphere facing away from the sun, scaled by tan(sun elevation over the ring plane) x RingsWinter

### 05-011 · Atmospheres, rings, comets

Eclipse shadow on the sky: SE computes EclipseShadowFar per sky fragment for each light, so a moon's shadow shows as a dark patch on the atmosphere seen from space, and during totality the horizon stays bright. The port dims the whole sky shell by one factor taken at the camera (or the ground point below it)

- Status: differs, effort **M**, auditor confidence high
- SE evidence: sky.glsl:255-260
- Port: crates/se_engine/src/system.rs:739-744
- Check: SE sky.glsl:255-260 evaluates EclipseShadowFar per fragment at FragPosS inside the NLights loop, while the port's update_sky_shells (system.rs:739-744) takes one eclipse factor at the camera/ground point and multiplies the whole shell's sun_irradiance by it, and sky.wesl has no shadow term beyond the sun-disc occluder mask.

### 05-016 · Atmospheres, rings, comets

Feature switches: SE has toggles and settings for this area: ToggleAtmospheres, ToggleClouds, ToggleAurora, ToggleCometTails, ToggleEclipses, ToggleSkyRingsShadows and TogglePlanetShine; Settings "Eclipse shadows", "Atmospheric extinction", "Ring shadows on sky", "Volumetric rings", "Volumetric rings quality", "Comet brightness", "Aurora brightness", "Aurora quality" and "Rings resolution"; main-def.cfg EnableEclipses, AuroraBright and CometTailBright; and the Filters toolbar can hide clouds. The port has none of these switches

- Status: missing, effort **M**, auditor confidence high
- SE evidence: strings_utf16 ToggleClouds ToggleAtmospheres ToggleAurora ToggleCometTails ToggleEclipses; main-def.cfg PlanetRendering EnableEclipses AuroraBright CometTailBright
- Port: none
- Check: SE evidence checks out (strings_utf16 ToggleAtmospheres/ToggleClouds/ToggleAurora/ToggleCometTails/ToggleEclipses/ToggleSkyRingsShadows/TogglePlanetShine and labels "Eclipse shadows", "Atmospheric extinction", "Aurora brightness", "Rings resolution" etc.; main-def.cfg EnableEclipses:103, AuroraBright:110, CometTailBright:111, ObjectFilters ShowAtmospheres/ShowClouds/ShowAurora/ShowCometTails), and the port has none: settings.rs saves only star/orbit/label/music/graphics-preset keys, no Show*/Toggle* or brightness keys anywhere in crates/, and the eclipse, aurora, comet and cloud code paths run unconditionally

### 05-008 · Atmospheres, rings, comets

Volumetric rings: with VolumetricRings (default true) SE ray-marches individual ring rocks close up: PBR, triplanar DefaultRingsRockTexture "SnowSE001", subrings with SubringLayerRotationAdjust, RingRockMaxSpacing, catalog RocksMaxSize/RocksSpacing, fading by RingRocksFadeStart/EndSizePix. It adds volumetric dust with self-shadow and gives the 2D ring a real Thickness that fades into the 3D rings. The port draws only a flat textured annulus, so close up there are no rocks, no dust and no thickness

- Status: missing, effort **L**, auditor confidence high
- SE evidence: rings_raymarch.glsl:62-388; main-def.cfg RingsRendering 138-152 and PlanetRendering VolumetricRings; strings_utf16 "Volumetric rings quality" "Draw 3D rings"
- Port: crates/se_engine/src/rings.rs:288; crates/se_engine/assets/shaders/rings.wesl
- Check: rings_raymarch.glsl (388 lines) ray-marches rocks with triplanar PBR textures (lines 107-205), dust with ECLDUST/ECLSTONES, and main-def.cfg:101 VolumetricRings true plus RingsRendering 141-148 configure it; the port draws only a flat annulus mesh (rings.rs:288 annulus, rings.wesl) and rocks_max_size_km/rocks_spacing/thickness_km are parsed but never read outside se_core rings.rs

### 05-012 · Atmospheres, rings, comets

Planetshine: SE's EnablePlanetShine (PlanetShineMode, PlanetShineBright, PlanetShineUpdateFrames, PlanetShinePickPerFrame) renders previews of nearby planets and lights bodies with their colour as a secondary light (e.g. earthshine on the Moon's night side), and the host planet lights its own rings (HostLightIndex in rings.glsl / rings_raymarch.glsl). The port has no secondary lighting: night sides get only a fixed 0.015 ambient, and rings are lit only by the star (sunlit face plus light passing through to the back face), with no light from the planet

- Status: missing, effort **L**, auditor confidence high, amended by the checker
- SE evidence: main-def.cfg PlanetRendering EnablePlanetShine..PlanetShinePickPerFrame; strings_utf16 "Planet shine" "Planet shine brightness" TogglePlanetShine; rings.glsl:66 HostLightIndex
- Port: none
- Check: main-def.cfg:105-109 EnablePlanetShine..PlanetShinePickPerFrame and the HostLightIndex branch in rings.glsl:20,66 / rings_raymarch.glsl check out, and the port has no planetshine (grep finds none); but night sides are not fully black (terrain.rs:1443 sets a constant ambient of 0.015, used at planet.wesl:1171), and unlit ring faces are not black (rings.wesl shows the back face by sunlight passing through the ring).

### 06-011 · Procedural generation

Planet class words use designed thresholds instead of SE's: procgen temperature frigid<130/cold<240/cool<273/temperate<310/warm<350/hot<600 K, pressure 1e-7/1e-3/0.1/10/500 atm, terra size 1e-3/0.05/0.5/2/10 Me, Neptune 6/12/30/60, Jupiter 30/150/1000/2500 Me, glacial/superglacial from ice cover; SE (already ported in surface_gen/climate) uses >1000/500/330/250/170/90 K from the temperature model, 1e-6/1e-3/0.1/10/1000 atm, 0.002/0.02/0.2/2/10 Me, Neptunes 4/10/25/62.5 Me, Jupiters 0.2/0.5/2.5/12.5 MJ, volatiles from ocean depth vs BumpHeight and never assigns glacial; the Star browser search even classifies catalog bodies with the designed temperature thresholds

- Status: differs, effort **S**, auditor confidence high
- SE evidence: FUN_14025f1c0; FUN_1404caaa0; FUN_1404cabf0; FUN_140505680; docs/formats/surface_params.md "Body classes"
- Port: crates/se_core/src/procgen/mod.rs:293-363; crates/se_engine/src/star_search.rs:285
- Check: Exe constants read from SpaceEngine.exe confirm SE's thresholds (FUN_1404caaa0 pressure 1e-6/1e-3/0.1/10/1000; FUN_1404cabf0 terra 0.002/0.02/0.2/2/10, Neptune 4/10/25/62.5, Jupiter 63.6/158.9/794.6/3972.9 Me = 0.2/0.5/2.5/12.5 MJ; FUN_140505680 depth 0.1/0.9 BumpHeight) and surface_gen.rs:322/399 already has them, while procgen/mod.rs:293-363 uses its own "designed" thresholds and star_search.rs:285 calls procgen::temperature_class for catalog bodies

### 06-006 · Procedural generation

Procedural star physical parameters are placeholders: every procedural star feeds the planet generator age 4.0 Gyr and [Fe/H] 0 (no C/O), mass L^(1/3.5) (red giants come out several solar masses), white dwarfs 0.6 Msun / 8000 km, neutron stars 1.4 Msun / 12 km, black holes 5-20 Msun from a name hash; SE carries each generated star's mass, radius, luminosity, Teff, metallicity, C/O and age into the system (FUN_1404f9210:116-129) and sets the system age from it capped by stellar lifetime (FUN_1404f7c40:105-138, FUN_1401dd7e0), so procedural planets' metallicity-dependent composition, tidal locking, life stages and atmospheres differ (separately, the port panel shows Age/Lifetime/Metallicity for no star at all)

- Status: partial, effort **M**, auditor confidence high, amended by the checker
- SE evidence: FUN_1404f9210:116-129,279-292; FUN_1404f7c40:105-138; eng-gui "Age / lifetime", "Metallicity"
- Port: crates/se_engine/src/system.rs:398-433
- Check: Port proc_system_input (crates/se_engine/src/system.rs:398-410) hard-codes age_gyr 4.0 and feh 0.0 and procedural_star_record (system.rs:415-433) sets mass L^(1/3.5), WD 0.6/8000 km, NS 1.4/12 km, BH 5-20 from name hash, while SE copies the generated star's mass/radius/Teff/metallicity/age fields into the system (FUN_1404f9210:116-129) and caps the age by lifetime (FUN_1404f9210:279-292 FUN_1401dd7e0, FUN_1404f7c40:105-138); the port only derives a lifetime-based system age for catalog stars (se_core/src/climate.rs catalog_system_age), but the panel clause is wrong: the port's info panel shows no Age/Lifetime/Metallicity for any star (no such field in se_engine/src), so that is a general UI gap rather than a procedural one

### 06-007 · Procedural generation

Rings on every kind of cold body missing: SE rolls rings for 60% of bodies (5% with record flag 0x165c bit 0) of any class with mass > 1e-7 Earth and temperature below 80 + 220 (log10 M + 5) / 4.4 K (max 300 K), so many terrestrial planets, dwarf planets and moons are ringed, with styles from ProbRingsElike/Thin/Narrow/Sparse/Contrast, plus 3% "debris rings" of up to 1000 small bodies on circular orbits (ProbRingsDebris); port gives rings only to gas giants (10%, 40% if cold) and ice giants (25%), NoRings for all solid bodies, no debris rings

- Status: differs, effort **M**, auditor confidence high
- SE evidence: FUN_140508600:16-37 (constants 0.6/0.05/80/220/4.4/300 at 0x1406d442c..); FUN_140500700:425-446 (DAT_14075f874, up to 1000); main-def.cfg PlanetGenerator ProbRings*
- Port: crates/se_core/src/procgen/mod.rs:1379-1395
- Check: Exe constants confirm FUN_140508600: chance 0.6 (0x1406d442c) or 0.05 (0x1406d4068, flag 0x165c bit 0), mass > 1e-7 (0x1406d3e38), T limit 80 + (log10M+5)*220/4.4 clamped 80..300, called from four body generators (FUN_140501620/140502710/140502eb0/140503780); config loader FUN_140280ec0:3100-3105 maps ProbRingsDebris to DAT_14075f874 used at FUN_140500700:425-446 with count capped at 1000; port assigns b.rings only in physics_giant (procgen/mod.rs:1379-1395, 10/40% gas, 25% ice) and writes NoRings for everything else (mod.rs:2285)

### 06-005 · Procedural generation

Procedural star population differs: SE draws stars from class tables built at start (mass/luminosity/radius per spectral class and luminosity class, FUN_1401d7300), converts O stars to Wolf-Rayet (ProbWolfRayetStar 0.05, FUN_1401d7190) and red giants to carbon/zirconium stars (ProbCarbonStar 0.08, FUN_1401d70c0), includes brown dwarfs (L/T/Y), subgiants, subdwarfs, bright giants and hypergiants, and solitary WD/NS/BH at the last level with separate galactic-kern rates (ProbSolitaryWD/NS/BH, ProbKernSolitary*); port uses a designed luminosity function with only class V dwarfs, G-M giants III/II/Ib, DA white dwarfs (8% of Mv>10) and fixed NS/BH densities, faint limit Mv 16 (no brown dwarfs)

- Status: differs, effort **L**, auditor confidence high
- SE evidence: FUN_1401d7300:144-156; FUN_1401d7190; FUN_1401d70c0; main-def.cfg SolarSystemGenerator ProbWolfRayetStar..ProbKernSolitaryBH; eng-gui "Wolf-Rayet star", "Carbon star", "Zirconium star", "Red subdwarf", "Blue hypergiant"
- Port: crates/se_core/src/star_octree.rs:19-70 (LF, FAINTEST_MAG), 603-623, 689-719 (star_for_mag)
- Check: FUN_1401d7190 (O-class -> WR class code by DAT_14075f824) and FUN_1401d70c0 (cool giant -> carbon/zirconium by DAT_14075f820) exist, main-def.cfg lists ProbWolfRayetStar, ProbCarbonStar, ProbSolitary*/ProbKernSolitary*, ProbBrownDwarf; port star_octree.rs uses a fixed LF to Mv 16, star_for_mag (689-719) gives only V dwarfs, M/K/G III/II/Ib giants and DA white dwarfs, and remnants at fixed per-pc3 densities (603-623) with no WR, carbon, brown-dwarf, subdwarf or hypergiant draws

### 06-009 · Procedural generation

Life generator is designed and narrow: SE (FUN_1404faf40, every system) gives organic life by volatiles class (ProbLifeOrgTerra 0.3 non-arid, Oceania 0.3, Desert 0.02 arid, IceWorld 0.05 subglacial on ice worlds > 1200 km, Giant 0.01 aerial) at 250-640 K (giants 150-400 K) and exotic life at 80-111 K (giants 60-120 K) with ProbLifeExo*, then panspermia between bodies of the system (ProbPanspEject*/Insert*, `Life { Panspermia }`), and the development stage (unicellular/multicellular/complex) from Gaussian age thresholds LifeOrgAge*/LifeExoAge* (FUN_1404c9d90); port gives life only with a liquid ocean at 268-332 K, 0.3-6 atm (35%, 6% exotic, Marine biomes), no desert/subglacial/aerial life, no panspermia, and catalog bodies never get generated life

- Status: differs, effort **L**, auditor confidence high
- SE evidence: FUN_1404faf40:84-282 (temperatures 250/640/80/111/150/400/60/120/1200 at 0x1406d58e8..); FUN_1404c9d90; main-def.cfg LifeGenerator 896-933; eng-gui "Life origin", "Abiogenesis", "Panspermia", "Subglacial", "Aerial"
- Port: crates/se_core/src/procgen/mod.rs:1697-1720; docs/formats/climate.md "Life generation ... not ported"
- Check: FUN_1404faf40 tests ProbLifeOrg*/Exo* per volatiles class with temperature windows (0x1406d59a0 = 400 K giant limit, 0x1406d5ab8 = 1200 km ice-world radius limit) and then panspermia via ProbPansp* (DAT_14075f8bc..8cc), matching main-def.cfg 896-933; port life is one rule (procgen/mod.rs:1695-1720: liquid water, 268-332 K, 0.3-6 atm, 35%/20%, 6% exotic, Marine biomes), no ProbLife/panspermia anywhere in crates/, and docs/formats/climate.md:237 says life generation is not ported

### 06-010 · Procedural generation

Binary planets and massive moons with barycentres missing: SE creates very massive moons and binary planets (ProbMassiveMoonGasGiant/Terra, ProbBinaryGasGiant/Terra, outer moons around binary planets ProbBinaryPlanetMoons) with a planet-moon barycentre as parent, e.g. SE place "An Island Terra" Body "RS 8514-491-6-169551-68 6" Parent "... 6-6.1", and "Nebula in the Night Sky" Body "... B5b" Parent "... B5"; port moon rules are designed (Moon-like large moon 25% for M >= 0.1 Me) and never make barycentres or binary planets

- Status: missing, effort **L**, auditor confidence high
- SE evidence: FUN_1404fdc90:806-823; FUN_140501990; FUN_140503220; save/places-def.cfg:436-437,468-469
- Port: crates/se_core/src/procgen/mod.rs:1912-2108 (gen_moons)
- Check: FUN_1404fdc90:806-823 builds a barycentre record whose mass is the sum of the two bodies (0x23f fields) with both as children, and SE's own places-def.cfg:436-437,468-469 has bodies parented to "...6-6.1" and "...B5" barycentres; port gen_moons (procgen/mod.rs:1912+) only adds a 25% large moon of mass ratio 10^-3..10^-1.7 for M >= 0.1 Me and no barycenter/binary code exists in procgen

### 06-012 · Procedural generation

Procedural star names do not follow SE's numbering: SE (FUN_1401c0180) names Milky Way stars "RS <galaxy 8514>-<field 2>-<level>-<node>-<index>" (places-def.cfg "RS 8514-491-6-169551-68"), other-galaxy stars "RS <RG code>-<field>-<level>-<node>-<index>", and cluster/nebula stars "RSC ..."/"RSN ..."; the port names Milky Way stars "RS <level>-<x>-<y>-<z>-<index>" and other galaxies' stars with a Morton-packed node id, so SE's star/place names lead to no star or a different one

- Status: differs, effort **L**, auditor confidence high, amended by the checker
- SE evidence: save/places-def.cfg:436,468; exe strings "%i-%i-%I64d-%i", "RS" names
- Port: crates/se_engine/src/procstars.rs:140-142; crates/se_core/src/procgen/mod.rs:255-274
- Check: FUN_1401c0180 formats Milky Way stars as L"RS %i-%i-%i-%I64d-%i" (galaxy 8514, a second field param[6] e.g. 491, octree level, 64-bit node, index; RSC/RSN variants for cluster/nebula stars), so the gap's "RS 8514-<level>-<node>-<index>" drops a field; the port's proc_star_name (procstars.rs:140) writes "RS <level>-<x>-<y>-<z>-<index>" and procgen::procedural_star_name treats "8514-491" as a galaxy code with a Morton node id, so SE names still don't resolve

### 06-017 · Procedural generation

Info panel lacks SE's derived body data: the composed class description ("Cold arid terra with life", built from the PlanetClassOrder* order and gender forms; the port computes the temperature word internally for surface generation but only shows "Tag (Class)"), ESI, internal-structure layers (Metallic core, Silicate/Carbide mantle, Water/icy envelope, Ice VI/VII, H, He), orbital resonances, Hill/influence sphere radius, Roche limit for satellites (the port computes one only for procedural rings), small-body dynamical group (Aten..Damocloid, also exported as asteroid Class) and life origin

- Status: missing, effort **L**, auditor confidence medium, amended by the checker
- SE evidence: eng-gui "ESI", "Internal structure", "Metallic core", "Orbital resonances", "Hill / influence sphere radius", "Roche limit for satellites", "Life origin"; exe table 0x140687090 (FUN_1401c1aa0, used by FUN_1404eb190 export); eng-gui PlanetClassOrderTemp..Life
- Port: none (procgen/mod.rs:672 describe() only for procedural summaries; crates/se_engine/src/hud.rs:92-110 shows tag and bulk only)
- Check: The strings exist in the exe (re/strings_utf16.txt "Metallic core", "Hill / influence sphere radius", "Roche limit for satellites", "Cubewano"; PlanetClassOrder* in strings_ascii; dynamical-group table 0x140687090 via FUN_1401c1aa0 used by FUN_1404eb190), not in eng-gui.cfg, which is only a 5 KB header; the port's Selection window (hud.rs:230-310) shows only the tag/raw class, mass, temperature, orbit and day lines; it computes temperature class words (surface_gen.rs TEMP_NAMES) and a Roche limit (procgen/mod.rs:1383, rings only) internally but never shows them; visibility raised because the composed class line appears every time a planet is selected

### 07-015 · Camera and time

Session state: SE saves the planetarium state (MoveMode, BindMode, RefBody/RefParent, position, orientation, velocity, TimeScale, Date) and restores it on Continue (first start from save-def.cfg: Saturn, Follow mode); port always starts 4 radii from Earth at the current time and persists only display settings

- Status: missing, effort **M**, auditor confidence high
- SE evidence: save/save-def.cfg Planetarium block; LastMode
- Port: crates/se_engine/src/camera.rs:199-214; crates/se_engine/src/settings.rs:44-62
- Check: save/save-def.cfg has LastMode and a Planetarium block (MoveMode 1, BindMode 1, RefBody "Saturn", RefParent "Sun", AbsPosition/RelPosition, Orientation, Velocity, TimeScale, Date); port place_at_start (camera.rs:199-214) always places the camera 4 radii from cfg.start_at (default "Earth", only changeable by the --start CLI arg, config.rs:52) and settings.rs Snapshot persists only display/audio/graphics settings, no camera, binding or time state

### 08-026 · Interface

Screenshots: SE F11 saves without the interface and Ctrl+F11 with it, into its screenshots/ folder in ScreenshotFormat (jpg by default, also png/dds/tga/tif); port F11 always captures the whole window including the egui interface, as PNG to Pictures/SpaceEngine-rs, with no Ctrl+F11 variant or format option

- Status: differs, effort **S**, auditor confidence high, amended by the checker
- SE evidence: eng-gui "Screenshot without GUI" "Screenshot with GUI"; tutorial "Pressing [Ctrl]+[F11] takes a screenshot with SpaceEngine's interface"; strings "screenshots/"
- Port: crates/se_engine/src/hud.rs:50-65
- Check: Exe has "Screenshot without GUI"/"Screenshot with GUI" actions, "screenshots/" folder and ScreenshotFormat "jpg" default, and tut-04-camera.se says Ctrl+F11 includes the interface, while port hud.rs:50-65 spawns Screenshot::primary_window() (egui included) as PNG into Pictures/SpaceEngine-rs; the "on-screen message" claim is unproven (ScreenshotComplete looks like an event name, no message string found).

### 08-028 · Interface

Seizure warning: SE shows a seizure warning dialog at startup (ShowSeizureWarning true, menu/warning.txt); port shows none

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg Interface ShowSeizureWarning; Scripts.pak menu/warning.txt; eng-gui "Seizure Warning"
- Port: none
- Check: main-def.cfg Interface ShowSeizureWarning true ("show seizure warning at startup", referenced by FUN_14000ce00) and Scripts.pak menu/warning.txt holds the red "Seizure Warning" page ending "Press Esc to continue"; grep of crates/, README and docs for seizure/warning.txt/epilep finds nothing.

### 08-018 · Interface

Markers overlay: SE has a Markers switch (ShowMarkers, off by default) with per-type ShowMarkers* for bodies, stars, clusters, nebulae and galaxies, drawn as MarkersSize 7 px filled discs placed in front of the body (body_mark.glsl); port has no marker switch or star/DSO markers and instead always draws a 4 px hollow ring at each labelled system body, which SE does not show by default

- Status: partial, effort **M**, auditor confidence high, amended by the checker
- SE evidence: body_mark.glsl:20-43; main-def.cfg ShowMarkers*, MarkersSize
- Port: crates/se_engine/src/overlay.rs:311-313
- Check: body_mark.glsl:20-43 draws a filled smoothstep-edged disc pushed 1.1 radii toward the viewer, and main-def.cfg:565,737-750 has MarkersSize 7 and ShowMarkers (default false) with per-type switches incl. stars/clusters/nebulae/galaxies; port overlay.rs:311-313 always strokes a 4 px (7 px selected) ring at every labelled non-star body with no marker setting (OverlaySettings has none), so with SE defaults the port shows rings SE does not

### 08-019 · Interface

Orbit filters and marks: SE has orbits off by default with per-type switches (ships, planets, moons, dwarf planets, dwarf moons, asteroids, comets, suns, barycenters), Crop orbits, orbit point marks (OrbitMarksMode off/selection/all), fading trails (OrbitTrailsLength) and major/minor line thickness; port shows orbits by default, picks them by its own size/distance rules (minor bodies only when selected) and draws thin uniform lines with no options, trails or marks

- Status: partial, effort **M**, auditor confidence high, amended by the checker
- SE evidence: main-def.cfg ShowOrbits*, OrbitMarksMode; eng-gui "Crop orbits" "Orbit marks"
- Port: crates/se_engine/src/overlay.rs:118-136
- Check: main-def.cfg:708-721 has OrbitMarksMode 1, OrbitTrailsLength, OrbitMajor/MinorThickness, ShowOrbits false plus nine per-type switches, eng-gui "Crop orbits" "Orbit marks"; port draw_orbits (overlay.rs:104-151) uses fixed size/distance rules, minor bodies only when selected, 1 px uniform-alpha gizmo lines, no marks/crop/per-type options, and its orbits default on (overlay.rs:36) while SE's default off

### 08-022 · Interface

Object filters: SE switches each class on/off (catalog/procedural galaxies, clusters, nebulae, stars, planets, procedural planets of catalog stars, spacecraft, atmospheres, clouds, water, aurora, comet tails, accretion disks, jets) from the Filters toolbar with cycling messages ("Real stars only", "All planets"...); port only has procedural galaxies/clusters/nebulae checkboxes

- Status: partial, effort **M**, auditor confidence high
- SE evidence: main-def.cfg ObjectFilters 664-688; eng-gui "Real & Procedural planets" "Procedural stars only"
- Port: crates/se_engine/src/procsystems.rs:169-171
- Check: main-def.cfg ObjectFilters 664-686 lists 19 Show* switches and the locale has the cycling messages ("Real stars only", "Procedural stars only", "All planets", "Real & Procedural planets"); the port's only object-class switches are the three procedural deep-sky checkboxes in procsystems.rs:169-171 (all other checkboxes are orbits/labels/flares/physics), no atmosphere/cloud/water/aurora/tail/disk/jet or catalog/procedural star/planet toggles.

### 08-025 · Interface

Locations dialog: SE's dialog (F6) has Current / Featured / My locations / Journey log tabs, picture thumbnails ("Load location to update picture"), an info pane, rename, share via copy URL / BB code / html / script code, paste from clipboard, import/export and the se:// protocol, visit/game/system dates; port's Places window is a filtered list with save and delete

- Status: partial, effort **M**, auditor confidence high
- SE evidence: rtti sePlacesBrowser seSLSortMode; eng-gui "Featured locations" "My locations" "Copy URL to clipboard" "Paste from clipboard" "Rename" "Register se:// protocol"
- Port: crates/se_engine/src/hud.rs:401-473
- Check: Locale has "Current", "Featured locations", "My locations", "Journey log", "Rename", "Load location to update picture", "Copy URL/BB code/html code to clipboard", "Paste from clipboard", "Import"/"Export", "Register se:// protocol" and the tutorial opens Locations with [F6]; port's Places window (hud.rs:401-473) is one filtered list with jump, save-current and delete of user places, no tabs, thumbnails, info pane, rename, clipboard sharing, import/export or se:// handling (grep finds no clipboard/se:// code).

### 08-027 · Interface

Display units: SE formats values in the user's units (Units: large distances ly or pc, so kly/Mly/Gly by default; medium AU or light-hours; small km/miles/nautical miles; temperature K/°C/°F; star temperature; pressure atm/bar/psi/inHg/mmHg/Pa) chosen in "Units of measurement"; port uses one fixed format that switches to kpc/Mpc/Gpc above 1000 ly and offers no unit choice

- Status: differs, effort **M**, auditor confidence high
- SE evidence: main-def.cfg Units 578-615; eng-gui "kly" "Mly" "Gly" "Units of measurement"
- Port: crates/se_core/src/units.rs:32-51
- Check: main-def.cfg Units 578-612 has UnitDistLarge/Medium/Small, UnitTemperature, UnitStarTemperature, UnitPressure and the exe has "Units of measurement", "Celsius", "Fahrenheit", "Pound per square inch", "Inch of Hg", "nautical mile"; port units.rs:32-51 has one fixed formatter switching to kpc/Mpc/Gpc above 1000 ly, hud.rs:280 hardcodes °C, and no unit setting exists anywhere in crates/.

### 08-023 · Interface

Solar system chart: SE's chart (F8) compares bodies in modes by radius/mass/density/temperature or a system map (ChartDisplayMode 4), with hierarchical layout (planets beside their star, moons below, barycenters as blue diamonds with blue links, green planet links), lit rotatable 3D bodies (ChartLightingAngle 35), toggles for stars/planets/dwarf planets/moons/dwarf moons/asteroids/comets/barycenters, min/max diameter/mass/density/temperature, zoom/scroll by mouse and WASD; port's F6 chart draws planets as flat dots on a log-distance line with moons listed below, no modes or options

- Status: differs, effort **L**, auditor confidence high
- SE evidence: main-def.cfg SolarSystemChart 636-651; rtti seChartMode cmFilterMode; tut-06-system-chart strings; eng-gui "Min diameter" "Lighting angle"
- Port: crates/se_engine/src/browser.rs:238-319
- Check: tut-06-system-chart.se (opened with [F8]) describes the hierarchical scheme, mouse zoom/scroll, right-drag rotation of bodies, Size/Mass modes, the Stars checkbox, blue barycenter diamonds and green planet links, and main-def.cfg 636-648 has ChartDisplayMode 0-4, Chart* toggles and ChartLightingAngle; port browser.rs:238-319 (bound to F6, which is SE's Locations key) draws only planets/dwarf planets >=400 km as flat coloured circles on a log-distance line with up to 9 moons >=100 km listed below, no stars, barycenters, modes, options or zoom (the WASD detail is not evidenced, rest stands).

### 08-024 · Interface

Wiki window: SE's wiki has tabs (General, Physical, Orbital, Atmospheric...) with full physical, orbital, atmosphere and life data, a rotatable 3D preview, a status line ("Real object"/"Procedural (generated) object"), discovery fields (Pioneer name, Discovery date, Discovery method), an editable description ("Enter description here") and wiki export; port's wiki shows only the DB description or a distance/radius/parent summary (some physical/orbital numbers appear in the Selection panel instead, and Pioneer/Date are parsed in locale.rs but never shown)

- Status: partial, effort **L**, auditor confidence high, amended by the checker
- SE evidence: rtti seWiki seWikiPageMode seWikiExportDialog; eng-gui "General" "Physical" "Orbital" "Atmospheric" "Pioneer name" "Discovery method"; tutorial "switch to the Physical tab", "rotate the planet preview"
- Port: crates/se_engine/src/wiki.rs:71-134
- Check: Locale has the wiki fields ("Status", "Real object", "Procedural (generated) object", "Pioneer name", "Discovery date", "Discovery method" with its method list, "General"/"Physical"/"Orbital"/"Atmospheric", "Enter description here", export scopes) and the tutorial confirms the Physical tab and rotatable preview; port wiki.rs:71-134 shows only the DB description or distance/radius/parent, but the separate Selection panel (hud.rs:232-345) already shows mass, Teff/Mv, average/day/night temperature, a/e/P, rotation period and day lines, and locale.rs parses Pioneer/Date without displaying them, so the gap is the tabbed layout, atmosphere/life data, 3D preview, status/discovery lines, editable description and export rather than all object data.

### 09-003 · Graphics and settings

Exposure compensation: SE lets the user brighten or darken the whole image in photographic stops (ExposureComp, range MinExposureComp -4 .. MaxExposureComp 42) with the < / > keys (IncExposure, DecExposure), reset with / (ResetExposure), typed on the Camera toolbar, and saved with each location (place key Exposure). Port's exposure is a fixed constant (log2 0.75); places.cfg Exposure is parsed but never applied

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg PostEffects ExposureComp MinExposureComp MaxExposureComp; eng-gui "Exposure compensation" "Increase/Decrease/Reset exposure compensation", tutorial "Press and hold [<] or [>] keys", "The [/] key ... reset"; def_generic_controller.cfg IncExposure DecExposure ResetExposure
- Port: crates/se_engine/src/camera.rs:175; crates/se_core/src/places.rs:267-268
- Check: main-def.cfg ExposureComp/MinExposureComp -4/MaxExposureComp 42, locale "Exposure compensation"/"Increase/Decrease/Reset exposure compensation" and tutorial "Press and hold [<] or [>]", exe actions IncExposure/DecExposure/ResetExposure (default keys . , / per .audit/work/07/default_keys_named.txt); the port's exposure is the constant 0.75f32.log2() (camera.rs:175) and places.rs:268 parses Exposure but nothing in crates/se_engine reads it

### 09-005 · Graphics and settings

Final colour correction: SE applies ColorCorrentionLDR after tone mapping with Brightness, Contrast, Saturation (per-pixel vibrance 1 + (S-1)(1 - pixel saturation), so grey pixels gain more than vivid ones) and Gamma settings, plus a Dithering on/off switch; port has no brightness/contrast/saturation/gamma settings and approximates Saturation 1.5 by a uniform post_saturation 1.35 (vibrance at pixel saturation 0.3), over-saturating vivid colours (nebulae, red/blue stars, flare tints) and under-saturating greys; the port always dithers (Bevy DebandDither on Camera3d) with no switch

- Status: differs, effort **S**, auditor confidence high, amended by the checker
- SE evidence: hsl.glh:31-54 ColorCorrentionLDR; hdr_final.glsl:184-204; main-def.cfg PostEffects Brightness Contrast Saturation Gamma, RenderingQuality Dithering; eng-gui "Brightness" "Contrast" "Color vibrance" "Gamma" "Dithering"
- Port: crates/se_engine/src/camera.rs:167-177
- Check: hsl.glh:33-53 ColorCorrentionLDR (contrast, brightness, per-pixel vibrance 1+(S-1)(1-sat), gamma) and hdr_final.glsl:186-204 confirm SE's side and the port only has a uniform post_saturation 1.35 at camera.rs:175, but the port does dither: Bevy 0.20 makes DebandDither::Enabled a required component of Camera3d (bevy_core_pipeline core_3d/mod.rs:104), so only the Dithering toggle is missing, not dithering

### 09-006 · Graphics and settings

Settings window: SE's settings dialog has tabs (Graphics with groups for quality, anti-aliasing, post-effects, tone mapping, lens flares, point sources, planets and volumetrics; Camera; Display; Projection / 3D; Controls; Interface; Sound) and a "Reset settings" button that restores defaults (except controls) and restarts. Port has one small Settings window (language, a few star and overlay sliders, FOV) plus a separate Graphics window with 6 quality controls and the lens-flare options, and no reset

- Status: partial, effort **M**, auditor confidence high
- SE evidence: eng-gui "Graphics" "Visual style" "Camera" "Display" "Projection" "Controls" "Interface" "Sound" "Camera settings" "Graphics quality" "Reset settings" "Really reset settings" "This will reset all settings except controls and restart SpaceEngine"
- Port: crates/se_engine/src/hud.rs:351-391; crates/se_engine/src/graphics.rs:202-260
- Check: Exe UTF-16 strings hold "Reset settings", "Really reset settings", "This will reset all settings except controls...", "Graphics quality", "Camera settings", "Visual style" (English UI text lives in the exe; eng-gui.cfg only overrides a few), while the port has only the small Settings window (hud.rs:351-391), the 6-control Graphics window (graphics.rs:217-260) and a key-help "Controls" box (hud.rs:476), with no tabs and no reset anywhere in crates/

### 09-004 · Graphics and settings

Photo modes and auto exposure: SE has three photo modes (PhotoMode 0 HDR, 1 Manual, 2 Auto; V key / SwitchPhotoMode, saved per location). Manual and Auto render at physical brightness (the real* calibrations: realStarSurfaceBright 1e9, realStarCoronaBright 1000, realEmissionBright 3 ... versus the hdr* set, ExposureCalibration 2, FrameDynamicRange 65535). Auto meters the frame with a luminance histogram (hdr_histogram.glsl: centre-weighted mask AutoExpMaskSize 0.5, AutoExpMaskValue 1, 10-90% percentiles, target median AutoExpTargetBright 0.35, comp limits 0..42) and adapts at AutoExpSpeedUp 5 / AutoExpSpeedDown 1, so stars vanish over a lit surface and sunspots show on the Sun. Port has only one fixed, HDR-like look; tutorials and places that switch the photo mode change nothing

- Status: missing, effort **L**, auditor confidence high
- SE evidence: hdr_histogram.glsl:15-67; prim_histogram.glsl; main-def.cfg PostEffects PhotoMode AutoExp* ExposureCalibration FrameDynamicRange, Lighting hdr*/real* 252-295; eng-gui "Photo mode" "Auto" "Manual" "HDR", tutorial "This is the 'auto exposure' mode"; def_generic_controller.cfg SwitchPhotoMode; place key PhotoMode
- Port: crates/se_engine/src/camera.rs:167-177; crates/se_core/src/places.rs:267
- Check: hdr_histogram.glsl:15-67 builds the centre-masked log-luminance histogram and main-def.cfg has PhotoMode 0/1/2, AutoExp* and the hdr*/real* Lighting calibrations, with tutorial strings describing auto exposure hiding stars and showing sunspots; the port has no photo mode or histogram metering (places.rs:267 parses PhotoMode, nothing uses it), and its README "Exposure" adaptation is contradicted by stars.rs:385-386 which deliberately never dims the background (only intergalactic-vision gain in galaxies.rs:1045-1061 adapts)

### 10-009 · Scripts, audio, spacecraft

Script input locks: UserMoveControl, UserRotationControl, UserTimeControl and UserStereobaseControl "Disabled"/"Free" lock the user's camera, time and stereobase controls while a lesson or scene plays, and Goto is refused while movement is locked ("Camera movement is limited, ignoring Goto command"). Port ignores them, so the user can fly away during scripted sequences

- Status: missing, effort **S**, auditor confidence high
- SE evidence: Scripts.pak (13 uses in tutorials); exe command table usermovecontrol, userrotationcontrol, usertimecontrol, userstereobasecontrol
- Port: crates/se_engine/src/scripting.rs:577-580
- Check: UserMoveControl/UserRotationControl/UserTimeControl/UserStereobaseControl appear 13 times (tut-00, tut-01, menu/Scene1.se, Scene1VR.se) and the exe has "Camera movement is limited, ignoring Goto command"; port has no parser case for them (se_script.rs test unknown_commands_are_kept shows UserMoveControl becomes Op::Other) and scripting.rs:577-580 just warns and skips, so input is never locked.

### 10-005 · Scripts, audio, spacecraft

Script waypoints: Waypoint "name" { Parent RadiusKm Color Shape FixedPosPolarKm/XYZ... }, DeleteWaypoint, ClearWaypoints, Select/approach triggers on waypoints, ShowLabelsWaypoints and WaypointsOpacity; SE draws coloured markers that tutorials 02, 03 and 05 send you through. Port logs "waypoints are not supported", draws nothing, and the triggers on waypoints time out, so the lessons skip ahead without the player ever flying to a marker

- Status: missing, effort **M**, auditor confidence high, amended by the checker
- SE evidence: Scripts.pak tutorials/tut-02-free-mov.se:165-240 (13 Waypoint uses); exe rtti seWaypointList; command table waypoint, deletewaypoint, clearwaypoints
- Port: crates/se_engine/src/scripting.rs:565-567
- Check: tut-02-free-mov.se:165-290 defines WP 1..WP 7 (Parent, RadiusKm, Color, FixedPosPolarKm, Shape "Circle") and triggers on Object "WP 1", and exe strings WaypointsOpacity, ShowLabelsWaypoints, FixedPosPolarKm, CIRCLE/TRIANGLE/DIAMOND/HEXAGON exist; port scripting.rs:565-567 skips Waypoint/ClearWaypoints/DeleteWaypoint and scripting.rs:461-467 warns that triggers on them time out; but only tutorials 02, 03 and 05 use waypoints (tut-04 has none), and the port times those triggers out after UNSUPPORTED_WAIT_S rather than hanging

### 10-007 · Scripts, audio, spacecraft

Script access to settings: SE's Set/SetU/SetForce/Get/Reset/Interpolate/WaitVar/SaveVars/RestoreVars work on every engine setting. The tutorials and DefaultSettings.se set about 150 of them (PhotoMode, ShowConstellations, ExposureComp, AmbientLighting, ShowClouds, ShowAtmospheres, OrbitMarksMode, ChartDisplayMode, Show*/ShowLabels*/ShowMarkers*/ShowOrbits* filters, magnitude limits, Unit*, LandLOD, Map*/Chart*, WaypointsOpacity). GotoLocation also applies PhotoMode, Exposure and Stereobase, and SetForce LessonPassed stores tutorial progress. Port links only ShowOrbits, LabelsMode, StarMagnLimit, TimeScale and FOV, keeps the rest as inert script variables, and ignores GotoLocation PhotoMode/Exposure and LessonPassed

- Status: partial, effort **M**, auditor confidence high
- SE evidence: Scripts.pak tutorials/DefaultSettings.se; exe command table set, setu, setforce, get, reset, interpolate, savevars, restorevars
- Port: crates/se_engine/src/scripting.rs:655-690,767-825,924-927
- Check: Tutorials plus DefaultSettings.se Set/SetForce/Interpolate/WaitVar 141 distinct variables (ShowClouds, ShowAtmospheres, Unit*, ShowCat*/ShowProc* ...); port get_var/set_var map only showorbits, labelsmode, starmagnlimit, timescale, fov (scripting.rs:676-705, LIVE_VARS), store the rest inertly, put LessonPassed in IGNORED_VARS (never persisted), Reset/Get fall to Op::Other, and goto_location (scripting.rs:767-825) never reads loc.photo_mode/exposure/stereobase although se_script.rs:643-646 parses them.

### 10-008 · Scripts, audio, spacecraft

Script triggers and their parameters: the port's Control/Move ignores Direction (7 tutorial checks for forward/back/left/right/up/down), Time ignores TimeRange, Object/Approach ignores LonLatRange, and ActivationDelay, Value and AnimateObject are not read. Several kinds are missing: Control/StopRotate, StopOrbit, GotoCenter, Horizon, SyncRot, Free, Track, Untrack, ResetFOV, SetTime, SetCurrentTime, Map* and Chart* (MapZoom used 10 times), and Input/Button, Input/JoyButton, Input/Custom. Missing kinds time out after 4 s instead of waiting for the user

- Status: partial, effort **M**, auditor confidence high
- SE evidence: exe strings Control/Rotate .. Input/Custom, TimeRange, ActivationDelay, AnimateObject, LonLatRange, Direction; tut-02-free-mov.se Direction rows; tut-03 TimeRange
- Port: crates/se_engine/src/scripting.rs:993-1001; crates/se_core/src/se_script.rs:569
- Check: Exe has Control/StopRotate..Untrack, Map*/Chart*, Input/Button/JoyButton/Custom and TimeRange/LonLatRange/ActivationDelay/AnimateObject; tutorials use Direction 7 rows in tut-02, TimeRange 2x and LonLatRange 1x in tut-03, Control/MapZoom 10x, MapFocus 4x, SetTime, SetCurrentTime, ChartRotate/Center; port's trigger_class lacks all of these (scripting.rs:993-1001), control/move checks only RangeKm distance, time trigger with only TimeRange has no time_jd and returns Yes at once (worse than "ignored"), approach ignores LonLatRange, and nothing reads Direction/Value.

### 10-010 · Scripts, audio, spacecraft

Engine exhaust rendering: SE ray-marches a volume in each engine's box (ship_engines.glsl, up to 128 steps) with a density shape per engine Type: Fusion (core + envelope with noise), Chemical, Ion, Ion2, Aerospike (15x2 grid of jets), Sphere, Square and SuperDraco_1atm. Quality follows ShipEngineQuality 0-3, brightness follows hdrShipEnginesBright/realShipEnginesBright 10 and the per-effect BrightnessHDR/BrightnessReal. Port draws one additive teardrop mesh for every type, with a multiplier per type, and the plumes come out as thin lines at chase distance

- Status: differs, effort **M**, auditor confidence high
- SE evidence: re/se_shaders/ship_engines.glsl:59-96; ship_Fusion_engine.glh, ship_Aerospike_engine.glh, ship_Sphere_engine.glh ...; main-def.cfg ShipEngineQuality, hdrShipEnginesBright, realShipEnginesBright
- Port: crates/se_engine/assets/shaders/ship_plume.wesl:44-60; crates/se_engine/src/ship.rs:658-672
- Check: ship_engines.glsl:80-105 ray-marches the engine box up to 128 steps with a per-Type exhaustDensity include (8 ship_*_engine.glh files, Aerospike NJets 15x2), ShipEngineQuality main-def.cfg:116; port draws one additive teardrop mesh (ship_plume.wesl:44-60) with a per-kind constant (ship.rs:661-667), uses only BrightnessHDR (ship_assets.rs:457) and ignores BrightnessReal/ShipEngineQuality, and the auditor's .audit/work/10/plume.png shows a narrow streak; minor note: hdrShipEnginesBright default is 1 (only realShipEnginesBright is 10).

### 10-011 · Scripts, audio, spacecraft

Hull material maps: SE's ship.glsl applies normal maps (NormMap, 17 materials), detail bump maps at two scales (DetBumpMap/DetBumpScale/DetBumpHeight, 18), specular maps and Blinn highlights (SpecMap/SpecColor/SpecPower, 16-18), BumpMap/BumpHeight, DiffColorScale, TwoSided, the gamma corrections and a Lambert/Hapke lighting mix. Port shades with Lambert plus a flat fill from the diffuse and emissive maps only, so hulls look flat and matte

- Status: missing, effort **M**, auditor confidence high
- SE evidence: re/se_shaders/ship.glsl:240-385; ShipsTex.pak *.sml keys NormMap, DetBumpMap, SpecMap, SpecPower
- Port: crates/se_engine/assets/shaders/ship.wesl:49-68
- Check: ship.glsl:240-385 does tangent-space NormMap plus two-scale DetailNormMap blending, Lambert/Hapke mix (AmbientParams.w), SpecMap with gamma fix and normalised Blinn highlight, and ShipsTex.pak's 19 materials carry DetBumpMap 18, SpecPower 18, SpecMap 16, NormMap 16 (not 17); the port parses spec_map/norm_map (se_core/src/ship.rs:515-519) but ship.wesl binds only diffuse+emissive and shades Lambert+fill with no tangents, and README "Not yet implemented" lists hull specular/normal maps

### 10-006 · Scripts, audio, spacecraft

Tutorial GUI hooks: HighlightWidget "Main toolbar/Wiki" { Time Color } (172 uses, makes a widget blink), SetWidgetStyle "Message box" { Pos } (14), ShowDialog "Settings" { Tab } / HideDialog / HideAllDialogs / HideAllToolbars (55), OpenMenu "Main/Planetarium/Tutorial" / CloseMenu, StarBrowserReset { SearchRadius MatchOnly }, EnterPlanetarium, EnterSinglePlayer, ContinueSession, ShowFlightSchool, OpenStoreOverlay, plus the GUI/Open/Close/LeftClick/RightClick/SelectElement/SelectObject/ChangeValue/LockPanel/HalfLockPanel/UnlockPanel/Action* triggers (78 waits in the tutorials). Port skips all of these; GUI triggers time out after 4 s, so the tutorials run without their guided steps

- Status: missing, effort **L**, auditor confidence high
- SE evidence: Scripts.pak tutorials/tut-00..07; exe command table highlightwidget, setwidgetstyle, showdialog, hidealldialogs, hidealltoolbars, openmenu, closemenu, starbrowserreset; exe strings GUI/Open .. GUI/ActionPrev
- Port: crates/se_engine/src/scripting.rs:577-580,993-1001
- Check: Tutorials use HighlightWidget 172x, SetWidgetStyle 14x, ShowDialog/HideDialog/HideAllDialogs/HideAllToolbars 56 lines, OpenMenu 8x and 78 "GUI/*" waits (all names present in exe strings); port parses them as Op::Other and only warns (scripting.rs:577-580), trigger_class marks every GUI/* kind Unsupported so it times out after UNSUPPORTED_WAIT_S=4 s (scripting.rs:42,993-1001), and README itself says these are logged and skipped; closing needs SE-named widgets across the whole UI.

### 10-012 · Scripts, audio, spacecraft

Flight HUD: seSpacecraftHUD has modes Off/Horizontal/Orbital/Docking/Warp HUD (ShipOffHUD..ShipWarpHUD) with 3D pointers for the nose, velocity, reference body, target, target delta-v, warp delta-v and boost velocity (ShipNoseVector, Ship*Vector, Ship*PointerColor, ShipHUDColor), and a ship panel (LockShipPanel). The panel shows Altitude/Elevation; Ship/Body/Rel/Surf/Air velocity; 1st/2nd cosmic velocity; Grav/Aero/Total accel and Thrust; g Force; Tidal force; Angular velocity/accel; per-group engine levels; Orbital parameters with Stable/Unstable orbit; Planet radius and Atmo height; and for warp Boost factor, Physical/Effective velocity, Efficiency and Arrival after. Port shows a text box with name, speed, throttle, warp state, altitude and g

- Status: partial, effort **L**, auditor confidence high
- SE evidence: exe rtti seSpacecraftHUD; exe actions ShipOffHUD, ShipHorizonHUD, ShipOrbitalHUD, ShipWarpHUD, ShipDockingHUD; gui "Orbital HUD" "Ship velocity" "1st cosmic vel" "g Force" "Stable orbit" "Boost factor" "Arrival after"; main-def.cfg Toolbars LockShipPanel
- Port: crates/se_engine/src/ship.rs:1270-1330
- Check: exe has rtti seSpacecraftHUD, utf16 actions ShipOffHUD..ShipDockingHUD and ShipNoseVector, ascii ShipHUDColor/Ship*PointerColor, main-def.cfg:617 LockShipPanel and utf16 strings "Orbital HUD" "Ship velocity" "1st cosmic vel" "g Force" "Tidal force" "Stable orbit" "Boost factor" "Arrival after"; port draw_flight_hud (ship.rs:1270-1330) is one egui text box (name, speed, throttle, warp, nearest-body altitude, g) with no HUD modes, no 3D pointers and no orbital/accel readouts

### 10-015 · Scripts, audio, spacecraft

Ship manager and multiple ships: seShipsManager has My ships / Other ships tabs and a Build menu by class (Astronaut, Probe, Shuttle, Starship, Planetship, Space station, Building, Planetary base) with Build, Build fleet, Destroy, Destroy all and a Nickname. It offers Take control, Control remotely and Teleport here, keeps many ships alive in the universe at once, and limits piloting to Flight simulator mode ("To pilot ships, switch to the simulator mode"). Port's Ships window lists designs and spawns one ship at the camera; there is only one controllable ship and no build, fleet, switching or teleport

- Status: partial, effort **L**, auditor confidence high
- SE evidence: exe rtti seShipsManager, SMTab, seShipsManagerBuildDialog, seSpacecraftArray; exe utf16 "My ships" "Other ships" "Take control of spacecraft" "Control remotely" "Teleport here" "Build fleet" "Destroy all"
- Port: crates/se_engine/src/ship.rs:1177-1268,400-466
- Check: exe rtti seShipsManager, SMTab, seShipsManagerBuildDialog, seSpacecraftArray and utf16 "My ships" "Other ships" "Take control of spacecraft" "Control remotely" "Teleport here" "Build fleet" "Destroy all" "Planetship" "Planetary base" "To pilot ships, switch to the simulator mode" are present; the port's `ship` command despawns every existing ship before spawning one at the camera (ship.rs:334-337) and the Ships window only has Fly/Enter/Remove ship (ship.rs:1240-1256), so there is one ship, no build menu, fleet, switching, remote control or teleport

### 10-016 · Scripts, audio, spacecraft

Warp bubble effect: while the warp drive runs, SE renders an Alcubierre warp bubble around the ship that lenses and blueshifts the background (einstein.glsl with WARP/ALCUBIERRE_METRIC, warp env maps). It is set by ShipWarpQuality 0-4, ShipWarpRadius, WarpPrecision and "Warp & refl. update interval". Port shows no visual change in warp beyond the faint Hyper plumes

- Status: missing, effort **L**, auditor confidence high
- SE evidence: re/se_shaders/einstein.glsl:4,89-90,289-297; main-def.cfg WarpRendering ShipWarpQuality, ShipWarpRadius, WarpPrecision; gui "Warp & refl. update interval"
- Port: none
- Check: einstein.glsl header lists the "Alcubierre warp bubble" and lines 89-90/289-297 implement it under WARP, main-def.cfg WarpRendering has ShipWarpQuality 3 / ShipWarpRadius 10 / WarpPrecision, exe utf16 has "Warp & refl. update interval" (an exe string, not eng-gui.cfg); the port has no Alcubierre/warp-bubble pass (only blackhole_lens.wesl) and warp only drives the Hyper plume group (ship.rs:1135).


## Visibility 2: niche

### 01-019 · Stars

catalog key Remove ignored: star .sc files use Remove "name" (24 times) before redefining a system, but the port only honours RemoveStar, so the HIPPARCOS entries stay as duplicates next to the new components (Albireo: csv Albireo and BET2 Cyg are still drawn besides Albireo Aa/Ac/B, the csv one 18 pc in front)

- Status: missing, effort **S**, auditor confidence high
- SE evidence: Catalogs.pak stars/Stars-bin.sc:662-663 Remove "BET1 Cyg"/"BET2 Cyg", stars/ExoPlanetsSuns-bin.sc:238-678
- Port: crates/se_core/src/catalog.rs:255-261
- Check: SE's star reader FUN_140475fb0:364-420 matches tag "Remove" exactly like "RemoveStar" (same 0x7fff removal record), and 24 Remove lines exist in stars/*.sc (Stars-bin.sc:662-663 BET1/BET2 Cyg); port load_sc (catalog.rs:255) only matches "RemoveStar", and add_star (catalog.rs:385-397) does not patch a csv star with a StarBarycenter, so csv "Albireo/BET1 Cyg" (100.5 pc) and "BET2 Cyg" stay beside the 118.2 pc barycentre

### 01-020 · Stars

black holes with luminous disks seen from afar: SE gives an accretion disk its own luminosity (AccretionDisk Luminosity/LuminosityBol, e.g. Sgr A*, 3C 273 S*, M51-ULS-1 B) so a lone hole can show as a point; port parses LuminosityBol but never uses it, leaves class X out of the starfield and draws nothing until the lens region is 1.5 px wide (holes with a luminous companion, like SS 433, do show via the companion)

- Status: missing, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: exe AccretionDisk keys LuminosityBol, LumBol; Catalogs.pak stars/BlackHoles.sc "SS 433"
- Port: crates/se_engine/src/stars.rs:285, crates/se_engine/src/blackholes.rs:145
- Check: Disk Luminosity/LuminosityBol are real AccretionDisk keys (exe strings 28555-28556; planets/BlackHoles.sc SS 433 B, 3C 273 S*, SagittariusA.sc Sgr A* Luminosity 1e5) and the port parses luminosity_bol (body_params.rs:398) but never uses it, while stars.rs:285 skips class X and blackholes.rs:45/263 draws nothing below 1.5 px; but SS 433 is a bad example since its A7 Ib companion SS 433 A is drawn at the barycentre, and SE's point use of the key is not decompiled

### 01-022 · Stars

pseudo lens flare: SE's preset also draws lens dirt and a starburst (DirtTexture, DirtBright, DirtDispersal, RaysTexture); port draws ghosts and halo only

- Status: partial, effort **S**, auditor confidence high
- SE evidence: Flares.pak *pseudo.cfg keys; exe RaysTexture, DirtTexture
- Port: crates/se_engine/assets/shaders/pseudo_flare.wesl, crates/se_engine/src/lensflare.rs:495-514
- Check: SE hdr_combine.glsl:231-240 samples LensDirt and a rotated LensRays texture, multiplies ghosts+halo by (rays + dirt) and mixes a bloom dirt mask by DirtBright, with Default pseudo.cfg setting RaysTexture LensRays2 and DirtTexture; port's pseudo pass (lensflare.rs:499-505, pseudo_flare.wesl) passes only ghosts, halo, chromaticism and lens colour, with no dirt or rays anywhere

### 01-023 · Stars

CenterOf ignored: SE ties a star or black hole to the centre of a galaxy or nebula (134 supermassive holes in BlackHoles.sc, SS 433 in the Manatee Nebula, Crab Pulsar's barycentre in the Crab Nebula); port uses only the star's own RA/Dec/Dist

- Status: missing, effort **S**, auditor confidence medium
- SE evidence: Catalogs.pak stars/BlackHoles.sc CenterOf, stars/NeutronStars-custom.sc CenterOf "Crab Nebula"
- Port: none
- Check: SE star parser FUN_140476e90:274-283,875-890 reads CenterOf, sets flag 0x20 and queues the name to tie the star to that object instead of its own coordinates (134 uses in BlackHoles.sc, Crab Pulsar, SpaceEngine.sc); port has no CenterOf handling (only listed in docs/formats/sc_vocabulary.md), so holes whose own Dist differs from their galaxy's (NGC 1023* 10.57 vs 9.7 Mpc, TON 618* 5.59 vs 3.31 Gpc) sit outside their galaxy

### 01-021 · Stars

point source settings: SE's Graphics "Point sources" group has Points scale (PointScale), Link points scale with window resolution (LinkScaleToScreenRes, 2x at 2160p), Stars overbright (PointOverbright, Inc/DecStarsOverbright keys), Flare bright, Flare decay, Desaturate dim stars and Planet points style (PlanetPointMethod); port's Settings has only mag limit, brightness, size, saturation and star points style, and star size does not follow screen resolution

- Status: missing, effort **M**, auditor confidence high
- SE evidence: main-def.cfg PointRendering PointScale, LinkScaleToScreenRes, PointOverbright, PointFlareBright, PointFlareDecay, PointDesaturation, PlanetPointMethod; exe "Points scale", "Link points scale with window resolution", "Stars overbright"
- Port: crates/se_engine/src/hud.rs:364-379, crates/se_engine/src/stars.rs:162-186
- Check: main-def.cfg:154-165 has PointScale, LinkScaleToScreenRes, PlanetPointMethod, PointOverbright, PointFlareBright, PointFlareDecay, PointDesaturation and strings_utf16.txt:2785-2796, 2952-2953 hold the Graphics "Point sources" labels and Inc/Dec stars overbright; port StarSettings (stars.rs:162-186) and the Settings panel (hud.rs:365-379) have only mag limit, brightness, size, saturation, point style and blur length, and no grep hit for any of these keys or a resolution-linked point scale

### 01-017 · Stars

Kerr-Newman black holes: SE traces geodesics in the Kerr-Newman metric (KerrSpin, KerrCharge), giving the flattened, shifted shadow and frame dragging of spinning holes; port uses Schwarzschild lensing, KerrSpin only moves the disk's ISCO, KerrCharge ignored

- Status: differs, effort **L**, auditor confidence high
- SE evidence: gr_geodesic.glh:174-210,280-354; gr_intersection.glh:57-61; exe KerrSpin, KerrCharge
- Port: crates/se_engine/assets/shaders/blackhole_lens.wesl, crates/se_engine/src/blackholes.rs
- Check: gr_geodesic.glh:174-210 KerrGetR2/KerrGetK (Kerr-Newman in Kerr-Schild coords using KerrA, KerrQ2) and 280-354 horizon/naked-singularity handling; port blackhole_lens.wesl:52-54 integrates only u''+u=3/2u^2 (Schwarzschild), KerrSpin used only in isco_rs (blackholes.rs:121-128,150), no KerrCharge anywhere in crates/.

### 01-018 · Stars

neutron stars and white dwarfs in the GR shader: SE renders their accretion disks and jets and the light-bent star surface (STAR define: surface temperature, radius in Rg, equator velocity v/c, WD metric scaling); catalog AccretionDisk blocks on pulsars and X-ray binaries (NeutronStars-bin.sc, Crab Pulsar); port handles only class X holes, neutron stars are a plain 10 km sphere with glare

- Status: missing, effort **L**, auditor confidence high
- SE evidence: einstein.glsl:2-12,49-52; gr_intersection.glh:641-648; Catalogs.pak planets/NeutronStars-bin.sc AccretionDisk
- Port: crates/se_engine/src/blackholes.rs:145
- Check: einstein.glsl:1-12 (neutron stars, white dwarfs with volumetric disk and jets, STAR define) and gr_intersection.glh:48-53,641-648 StarSurface with starTemp/starRadius/starEqSpeed/WD scaling; Catalogs.pak planets/NeutronStars-custom.sc "Crab Pulsar" and NeutronStars-bin.sc pulsars carry AccretionDisk blocks; port reads AccretionDisk only for StarKind::BlackHole (blackholes.rs:145-155), neutron stars are an ordinary star body (12 km procedural, system.rs:424).

### 02-014 · Star clusters

Cluster display filters: SE has ShowCatClusters and ShowProcClusters and a ToggleClusters cycle (Clusters disabled / enabled / Real clusters only / Procedural clusters only / All clusters); port only has ShowProcClusters, catalog clusters cannot be hidden, no toggle action

- Status: partial, effort **S**, auditor confidence high
- SE evidence: main-def.cfg ShowCatClusters, exe strings ToggleClusters "Real clusters only"
- Port: crates/se_engine/src/procsystems.rs:170, crates/se_engine/src/procgalaxies.rs:49-65
- Check: SE: main-def.cfg:668-669 ShowCatClusters/ShowProcClusters, strings_utf16.txt:2871-2875 "Clusters disabled/enabled/Real clusters only/Procedural clusters only/All clusters", :4170 ToggleClusters action (bound in def_*controller.cfg), FUN_140499ce0 filters by globals 0x1419fab60/0x1419fac60; port has only the ShowProcClusters checkbox (procsystems.rs:170, proc_galsys.rs:197), no ShowCatClusters or show flag in clusters.rs and no toggle action

### 02-015 · Star clusters

NStars/Type handling: SE treats NStars 0 as "no generated members" (Pleiades, Hyades, Trapezium, M 24) and ignores any other NStars (class init zeroes it; open nodes always use the 16384 cap), Type Part (M 24) gets no members; port uses NStars > 0 as the member count (Praesepe 50, Mon R2 100, NGC 265 1000), handles NStars 0 like a missing key (luminosity-based count), relies only on a ">= 15 catalog stars inside the radius" skip for every cluster, and builds Part (M 24, radius 300 pc) as an Open cluster with up to 3000 generated members

- Status: differs, effort **S**, auditor confidence high, amended by the checker
- SE evidence: FUN_14047fcb0 (NStars==0 flag), FUN_1404826a0 (+0x48 = 0), FUN_140483080 (KERN/GLOBULAR/OPEN/PART), FUN_140543c70 (Part = 0 stars)
- Port: crates/se_engine/src/clusters.rs:351-360,373-382
- Check: SE verified: FUN_14047fcb0:717 NStars==0 sets flag +0x78/1, FUN_1404826a0:94 zeroes +0x48 for Kern/Glob/Open, FUN_140543c70 Part(4) -> 0 stars and Open -> 0x4000 cap; port clusters.rs:373-376 uses NStars>0 as count and treats NStars 0 like absent (luminosity count), while the >=15-catalog-stars skip (351-360) applies to every cluster, not as an NStars-0 substitute; port builds every non-"glob" Type (Part) as Open, and Catalogs.pak has no Kern clusters, so the Kern half of the claim is moot for catalog clusters

### 02-016 · Star clusters

Galaxy GlobClusters key: SE uses a catalog galaxy's GlobClusters value (25 galaxies) as its globular-cluster count and only computes it when absent; port always uses the luminosity formula

- Status: missing, effort **S**, auditor confidence medium
- SE evidence: FUN_14016bef0 key GlobClusters, FUN_14049cf40 line 132
- Port: crates/se_core/src/proc_galsys.rs:470-486
- Check: SE: FUN_14016bef0:550-555 reads GlobClusters into loader +0x8c, which is runtime galaxy +0x64 (same 0x28 shift as Type +0x30->+8 and Lum +0x44->+0x1c), and FUN_14049cf40:132-158 computes the count from luminosity only when that field is < 0; 25 catalog galaxies set it (M 31 450, M 87 14660, LMC 50, Sag dSph 0, M 32 0 ...); port proc_galsys.rs:470-486 always uses the luminosity/size formula and no crate reads GlobClusters

### 03-016 · Galaxies and nebulae

Quasars: SE reads `Quasar` catalog entries as galaxies of their Type with a quasar flag (0x800) and draws them like galaxies; port draws them as bluish point sprites (profile 3) and excludes them from elliptical volumes and intergalactic vision

- Status: differs, effort **S**, auditor confidence low
- SE evidence: FUN_14016b430 ("Quasar" -> FUN_14016bef0, flag /= 0x800); Catalogs.pak galaxies/BlackholeHosts.sc (22 Quasar, Type Sb/E2)
- Port: crates/se_engine/src/galaxies.rs:309,324-326; ellipticals.rs:146
- Check: FUN_14016b430 parses Quasar blocks with the same FUN_14016bef0 as Galaxy and only ORs flag 0x800 (exe strings use it as the "Quasar" type label); BlackholeHosts.sc quasars carry Type Sb/E0/E2 and Radius like galaxies, and SE shaders have no quasar-specific galaxy path, while port galaxies.rs:309 gives them profile 3 point sprites with a fixed blue colour (type_color), and skips them in ellipticals.rs:146, galaxies.rs:1036 and procsystems.rs:213; how SE uses 0x800 when drawing stays unproven, so confidence remains low

### 03-013 · Galaxies and nebulae

Ray-marched nebula parameters unused: Randomize (uRandomize offsets the spiral noises; non-zero in 35 of 112 models and set per nebula in the catalog, e.g. Skylias Stellar Nursery), ExpansionBegin/ExpansionDuration (52 models expand over 500 years), NebScaleCoef/NebScaleAbs*, Star_field_scale are read but ignored, so nebulae sharing a pooled model look identical and never expand

- Status: missing, effort **M**, auditor confidence high
- SE evidence: nebula_raymarch.glsl SpiralNoiseC2/SpiralNoise3D_2 (p += uRandomize*10), NebulaScaleFunc; raymarch_common.glh uRandomize, uParamsTime; Nebulae-raymarch.pak keys
- Port: crates/se_engine/src/nebula_raymarch.rs; docs/formats/nebula_raymarch.md section 4 ("read but unused")
- Check: SE nebula_raymarch.glsl adds uRandomize*10 in SpiralNoiseC2/SpiralNoise3D_2 and applies NebScaleCoef plus NebScaleAbsBoth/X/Y in NebulaScaleFunc, Nebulae-raymarch.pak has 35 non-zero Randomize and 52 ExpansionDuration models, catalog sets Randomize per nebula (Skylias Stellar Nursery); port nebula_raymarch.rs packs no Randomize or expansion, packs NebScaleCoef into P_THICK.xy but galaxy_nebula_raymarch.wesl only reads P_THICK.zw, and docs/formats/nebula_raymarch.md:57 lists them as unused

### 03-015 · Galaxies and nebulae

Stars inside nebulae: SE generates stars belonging to nebulae (names "RSN %u-%u-%I64d-%u-%u-%u", "RSN %u-%u-%u"), switched per catalog nebula by GenerateStars (default on for Diffuse); port has no RSN stars and ignores GenerateStars

- Status: missing, effort **L**, auditor confidence medium
- SE evidence: re/strings_ascii.txt:749-750,764; FUN_140172400 ("GenerateStars", flag default = type Diffuse); Catalogs.pak nebulae/SpriteBambus.sc GenerateStars false
- Port: none
- Check: RSN name formats exist in re/strings_utf16.txt:749-750,764,767 (cited file was strings_ascii, wrong but the strings are real) and FUN_140172400 sets a per-nebula flag defaulting to Type==Diffuse and overridden by GenerateStars (SpriteBambus.sc sets it false on 8 Planetary nebulae); port has no RSN names, no GenerateStars key and no stars generated inside nebulae (procstars/proc_galsys)

### 04-017 · Planet surfaces

Ice specular: SE gives ice its own glint (SpecBrightIce, SpecPowerIce, RoughnessIce; Earth "SpecBrightIce 0.65") masked by the colour map's alpha when DiffMapAlpha is "Ice" (Europa, Triton, Pluto, Charon); the port ignores the ice keys and the Ice alpha mask, its only ground glint is a generic sheen from the biome sample layers' roughness (partial)

- Status: missing, effort **S**, auditor confidence high, amended by the checker
- SE evidence: planet.glsl:930 specTer = SpecParams.x * roughMap; exe strings "SpecBrightIce SpecPowerIce RoughnessIce", 'DiffMapAlpha "Ice"'; SolarSys.sc Europa/Pluto/Charon
- Port: crates/se_core/src/planet_tiles.rs:263
- Check: SE's AICE path sets specTer = SpecParams.x (ice specular bright) * roughMap (planet.glsl:133, 930, 1169) and SolarSys.sc sets DiffMapAlpha "Ice" on Europa, Triton/Pluto, Charon and SpecBrightIce 0.65 on Earth; the port parses only DiffMapAlpha "Water" and SpecBrightWater (planet_tiles.rs:263-264), but its ground does get a generic GGX sheen from the biome sample layers' roughness (planet.wesl:1172-1177), so it is partial, not wholly missing

### 04-018 · Planet surfaces

Moon close-up tiles never load: Moon-Closeups-KL.pak adds sparse pos_x tiles around the Apollo sites (Surface-KL levels 6-7, Bump-KL levels 5-7, beyond the main paks' Surface 5 / Bump 4); the port finds a layer's deepest level by probing tile 0_0 of pos_x, which these sparse levels lack, so it stops at Surface 5 / Bump 4 and has no per-tile fallback for sparse deeper tiles

- Status: missing, effort **S**, auditor confidence high, amended by the checker
- SE evidence: data/textures/planets/Moon-Closeups-KL.pak (pos_x/6_15_33 ... 7_*)
- Port: crates/se_core/src/planet_tiles.rs:214-226
- Check: Moon-Closeups-KL.pak holds 174 sparse pos_x tiles (Surface-KL 15/22/50 and Bump-KL 15/22/50 at levels 5/6/7) while the main paks stop at Surface level 5 and Bump level 4, and TileLayer::discover (planet_tiles.rs:214-226) stops at the first level whose pos_x 0_0 tile is missing, so the port never uses Surface levels 6-7 or Bump levels 5-7 (terrain.rs:877,1238 clamp to max_level, no sparse fallback)

### 04-020 · Planet surfaces

Water display toggle: SE's ShowWater object filter (key ToggleWater) hides and shows oceans with a fade (WaterSurfColor.a "water on/off animation"); the port has no such toggle

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg ObjectFilters ShowWater; def_generic_controller.cfg ToggleWater; planet.glsl:129,927
- Port: none
- Check: main-def.cfg:680 ShowWater, def_generic_controller.cfg:229 ToggleWater, exe strings ShowWater/ToggleWater and planet.glsl:129,927 (WaterSurfColor.a multiplies the water mask as the on/off animation) exist; no water/ocean toggle or filter anywhere in crates/ or the README.

### 04-015 · Planet surfaces

Brown dwarfs: SE draws them with tg_browndwarf_* (gas-giant bands from GetGasGiantCloudsColor and a Planck thermal glow, mixed with solar granulation by erosion); the port renders them as a plain star photosphere (limb darkening and granulation, blackbody tint at 0.8 Teff, no bands)

- Status: differs, effort **M**, auditor confidence medium, amended by the checker
- SE evidence: tg_browndwarf_color.glsl, tg_browndwarf_glow.glsl, tg_browndwarf_height.glsl
- Port: crates/se_engine/assets/shaders/star_surface.wesl
- Check: SE's tg_browndwarf_color/glow/height mix GetGasGiantCloudsColor/HeightMapCloudsGasGiant bands with the solar GlowMapSun/HeightMapSun by erosion, but the port spawns brown dwarfs with the generic StarSurfaceMaterial (system.rs:265-279) whose spot coverage is 0 below 3000 K, so they get granulation and limb darkening only, no sunspots as the gap said

### 04-016 · Planet surfaces

Catalog asteroid and comet models (Eros, Itokawa, 67P, Arrokoth): SE uses the asteroids.sml materials (NormMap, DetDiffMap/DetBumpMap detail textures with scales and magnitudes, SpecColor/SpecPower, DiffColor tint, Hapke); the port draws albedo-only Lambert with flat ambient, no normal or detail maps, and DDS textures (67P_albedo.dds, 67P_normal.dds) fall back to white

- Status: partial, effort **M**, auditor confidence high
- SE evidence: textures/planets/Asteroids/asteroids.sml (docs/formats/planet_tiles.md section 8)
- Port: crates/se_engine/assets/shaders/model.wesl:42-50; crates/se_engine/src/models.rs:81
- Check: asteroids.sml defines DiffColor/DiffColorScale, SpecColor/SpecPower, NormMap and DetDiffMap/DetBumpMap with scales and mags for Eros, Itokawa, 67P, 1I (and ship.glsl has Hapke params), while model.wesl is Lambert * texture + 0.02 ambient, body_models.rs:76-80 sets base_color white, and load_diffuse uses the image crate with only png/jpeg features so 67P's .dds maps fall back to white

### 04-019 · Planet surfaces

Water rendering model: SE's sea is the terrain pass's flat-water term plus a near-camera 3D water mesh cross-faded by distance (WaterParams.z), shaded with Cook-Torrance, ambient and per-channel WaterFogColor, with no sky reflection, refraction or Snell's window; the port draws a see-through sea mesh everywhere that reflects the sky table and shows Snell's window from below, so seas and lakes look different from the surface

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: water.glsl:348-355,400-403,580-636; planet.glsl:1243-1274
- Port: crates/se_engine/assets/shaders/planet.wesl:1195-1226,1358-1415
- Check: water.glsl:348-355 discards 3D water past 1/WaterParams.z and 545-636 shades it with Cook-Torrance sun specular plus diffuse sky irradiance and WaterFogColor attenuation (no reflect/refract anywhere in water.glsl), while the port's water_fragment (planet.wesl:1358-1415) is built per node at every distance (terrain.rs:1112, no fade) and adds sky_seen(reflect(-v,nw)) and a Snell's-window refraction branch.

### 05-019 · Atmospheres, rings, comets

Eclipse model details: SE allows up to 8 casters per light (MAX_ECLIPSES), shapes the penumbra with a smoothstep and a shadow gamma of 1/1.6, scales it by the eclipse intensity AmbientColor.a, uses ellipsoid-scaled fragment and light positions, and lets stars (e.g. binary companions) cast shadows. The port uses at most 4 non-star occluders with the exact covered-disc area and no gamma or intensity, and tints umbras of atmospheric casters red (0.1, 0.03, 0.006), which SE's shader does not do

- Status: differs, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: eclipse_common.glh:8-110
- Port: crates/se_engine/assets/shaders/eclipse.wesl:6-61
- Check: eclipse_common.glh confirms gamma 1/1.6, a smoothstep penumbra, MAX_ECLIPSES 8 (planet/sky/water/ship.glsl), scaling by AmbientColor.a (planet.glsl:1082, sky.glsl:259) and ellipsoid-scaled positions (FragPosEll/Ellipsoid), with no red tint; the port (terrain.rs:155-203, eclipse.wesl) keeps 4 occluders, uses the exact overlap area, filters out stars (`!b.star`) and tints umbras of atmospheric casters with (0.1,0.03,0.006). But gas giants are already casters in the port, so only stars are missing, and the flat penumbra comes from SE's smoothstep shape plus gamma, not just the gamma

### 05-022 · Atmospheres, rings, comets

Eclipse totality mask: SE's overlay "Eclipse totality mask" (ToggleEclipseMask) paints the umbra on surfaces blue (SurfParams1.x). The port has no such overlay

- Status: missing, effort **S**, auditor confidence high
- SE evidence: planet.glsl:1377-1380; strings_utf16 "Eclipse totality mask" ToggleEclipseMask
- Port: none
- Check: planet.glsl:1377-1380 adds SurfParams1.x to blue where EclipseMask<=0 (SurfParams1 comment line 134 "eclipse shadow mask"), toggled by main-def.cfg ShowEclipseMask false / ToggleEclipseMask / strings_utf16 "Eclipse totality mask"; the port has eclipse shadows (eclipse.wesl, README:176) but no grep hit for an eclipse/totality mask overlay or toggle in crates/

### 05-018 · Atmospheres, rings, comets

Glowing atmospheres: SE's THERM path adds the atmosphere's own glow (GlowColorAtmo), scattered as if lit from the planet's centre, on the sky and over the surface, for hot planets and lava worlds. The port has no atmosphere glow

- Status: missing, effort **M**, auditor confidence high
- SE evidence: sky.glsl:225-231; planet.glsl:1004-1015
- Port: none
- Check: sky.glsl:225-228 (THERM: Inscatter = inscatterSky(0)*GlowColorAtmo with EyeMuS=0) and planet.glsl:1004-1014 (inscatterGround(0)*GlowColorAtmo) are as cited; the port's sky.wesl/atmosphere_common.wesl have no glow or emission term (only planet.wesl surface lava emission and city lights), and GlowColorAtmo's CPU-side value still has to be reverse-engineered from the exe

### 05-020 · Atmospheres, rings, comets

Oblate bodies in atmosphere and shadows: SE's sky (Ellipsoid), eclipse (EllipsGrav) and ring-shadow (Oblateness) code uses the planet's ellipsoid, e.g. Saturn's flattened shadow on its rings. The port treats all bodies as spheres

- Status: missing, effort **M**, auditor confidence high
- SE evidence: sky.glsl:188,257; planet.glsl:872,1047,1066; rings.glsl:66; rings_raymarch.glsl:225
- Port: none
- Check: The cited lines use the ellipsoid as claimed (sky.glsl:188 FragPos*Ellipsoid, :257 lightPos*Ellipsoid; planet.glsl:872/1047/1066 EllipsGrav; rings_raymarch.glsl lightPos.g*=Oblateness.g in MainPlanetShadow), and se_engine never reads the catalog oblateness that se_core parses (catalog.rs:482), so sky, eclipse and ring shadows are all spherical; it overlaps 04-002 (the oblate body mesh) and 05-009 (ring shadow), and is best fixed together with 04-002

### 05-021 · Atmospheres, rings, comets

Auroras: SE draws an aurora as particle sprites (NorthParticles/SouthParticles) displaced by Fbm noise, with the upper colour taken from noise, moving rays, AuroraQuality (TEX_NOISE vs DETAIL), AuroraBright and photo-mode realAuroraBright 3. Its shader has no day/night mask. The port draws mesh curtains with its own noise, masks them to the night side and multiplies brightness by 1.5

- Status: differs, effort **M**, auditor confidence medium
- SE evidence: aurora.glsl:138-251; main-def.cfg AuroraQuality AuroraBright, Lighting realAuroraBright
- Port: crates/se_engine/assets/shaders/aurora.wesl:62-92; crates/se_engine/src/aurora.rs:173-182
- Check: aurora.glsl:126-251 is a sprite shader (VertexPos sprite centres, Fbm4-displaced, top colour from Fbm4 noise, FbmTex brightness drifting with ScaleTime.w, TEX_NOISE/DETAIL variants) with only a distance clip via SunVecClip and no day/night term, catalog writer FUN_1404eb190 emits NorthParticles/SouthParticles, and main-def.cfg has AuroraBright, AuroraQuality and realAuroraBright 3; the port's aurora.wesl:62-92 builds mesh curtains with its own 1D value noise and a night-side smoothstep discard, aurora.rs:180 multiplies bright by 1.5, and no AuroraBright/AuroraQuality/realAuroraBright or particle count is read anywhere in crates/

### 06-016 · Procedural generation

StarGenerator settings not honoured: SE drops procedural stars brighter than StarMaxAppMagn 10 seen from the Sun (port hard-codes 8.5 within 2.5 kpc, so mag 8.5-10 stars are duplicated by catalog stars), keeps procedural stars at least StarMinDistNormal 2.5 / StarMinDistBrownDwarf 2 / StarMinDistRemnant 3 pc from the Sun (port: no minimum) and scales density by StarDensityInGalaxy/StarDensityInCluster (port: not read)

- Status: differs, effort **S**, auditor confidence high
- SE evidence: main-def.cfg StarGenerator 966-978; FUN_140014900, FUN_1405945e0 (StarMaxAppMagn), FUN_14049b390 (StarDensityInGalaxy), FUN_14047fcb0 (StarDensityInCluster)
- Port: crates/se_engine/src/procstars.rs:290-293
- Check: FUN_140280ec0 binds StarMaxAppMagn/StarMinDist*/StarDensityIn* to DAT_14075f950..93c, which are read by FUN_1405945e0 (mag limit, squared min distances), FUN_140014900, FUN_14049b390 (galaxy density) and FUN_14047fcb0/FUN_140543c70 (cluster density); the port's only exclusion is the hard-coded `d < 2500 && m_app < 8.5` in procstars.rs:288-293 and no crate reads any StarGenerator key or applies a minimum distance from the Sun

### 06-013 · Procedural generation

Rogue planets (planemos) missing from the star field: SE makes 45% of last-octree-level systems (70% in the galactic kern) planemo systems, a free-floating planet with its own moons and rings (CreateRoguePlanetSystem); the port's procedural field holds only stars and remnants

- Status: missing, effort **M**, auditor confidence high
- SE evidence: FUN_1404f7c40:76-78 -> FUN_1404fc280; FUN_1401d7300:152-156 (ProbPlanemo, ProbKernPlanemo); eng-gui "Planemo"
- Port: crates/se_core/src/star_octree.rs:519-625 (no planemo kind)
- Check: FUN_1404f7c40 sends star type 0x6000 to FUN_1404fc280 ("seSolarSystem::CreateRoguePlanetSystem" strings) and main-def.cfg:870/874 has ProbPlanemo 0.45 / ProbKernPlanemo 0.7 "probability of planemo (rogue planet) ... at the last octree level"; the port's StarKind (star.rs:8) has no planemo kind and star_octree.rs makes only stars, neutron stars and black holes (catalog Planemo.sc objects load, procedural ones do not exist)

### 06-014 · Procedural generation

Supermassive black hole systems missing: SE builds a dedicated system for any black hole of 400+ solar masses (CreateSupermassiveBlackHoleSystem, FUN_1404fb950), adding int(3 log10 M) (up to 50) procedural stars on Keplerian orbits around it (about 19 extra for Sgr A*); the port gives black holes no generated companions at all

- Status: missing, effort **M**, auditor confidence medium, amended by the checker
- SE evidence: FUN_1404f7c40:68-75 -> FUN_1404fb950:85-285
- Port: none (no "supermassive" code in crates/)
- Check: FUN_1404f7c40 sends a black hole (type 0x5800, sub 0x680) with mass >= DAT_1406d59a0 = 400 to FUN_1404fb950, which adds int(3*log10(M)) (max 50) stars of type 3 on orbits (FUN_1405eb800 is a log), e.g. ~19 for Sgr A* (planets/SagittariusA.sc MassSol 4.31e6); nothing in the decompile ties it to galaxy centres, and the port has no such generator (procgen.md: black holes get no planets)

### 06-015 · Procedural generation

Generator seeds are not SE's: SE seeds the system from bits of the star's position (system_gen.md, used for catalog and procedural stars) and every body from the system seed and its index (stars seed*0x2e3+0x1486 / seed*0x234+0x1cdb, planets idx*0x5b7104ac - n*0x32491c1d + 0xcdb6e3e3 ^ seed, asteroids seed*0x619 + n*0x2cd + p*0x1e4), which also drive the surface generator and preset choice; port seeds procedural systems from a name hash and their bodies' surface_gen from name hashes, so SurfStyle/presets differ even where rules match

- Status: differs, effort **M**, auditor confidence high
- SE evidence: FUN_1404f9210:109-114,190-194; FUN_140500700:305-309; FUN_1404fd2a0:259-262; docs/formats/surface_params.md "Seeds"
- Port: crates/se_engine/src/system.rs:410; crates/se_engine/src/system.rs:309 (surface_gen::catalog_input name seeds)
- Check: Decompiles confirm SE's index-based body seeds (FUN_1404f9210:109-114 seed*0x2e3+0x1486 / seed*0x234+0x1cdb; FUN_140500700:305-309 idx*0x5b7104ac - n*0x32491c1d + 0xcdb6e3e3 ^ seed; FUN_1404fd2a0:259-262 seed*0x619+n*0x2cd+p*0x1e4), while the port's materialize_star (system.rs:388-410) seeds procgen with proc_surface::name_seed(name) and loads the result as catalog script, so its bodies get catalog_seeds name hashes (SE's rule for catalog bodies only, FUN_1404652f0), not system-seed+index seeds; system_gen::system_seed (position seed) exists but only drives the catalog-system draws in system_gen.rs (age, orbit angles)

### 06-018 · Procedural generation

Only one cloud layer per procedural body, and only one is rendered: SE gives eligible terrestrial atmospheres a random 1..PlanetCloudsTerra (default 3) cloud layers and its other branch 1 or 2 hard-coded layers (PlanetCloudsGasGiant is read from config but no code uses it); the port's procgen emits at most one Clouds block, and terrain.rs draws only one layer even for catalog bodies with several

- Status: partial, effort **M**, auditor confidence medium, amended by the checker
- SE evidence: main-def.cfg PlanetGenerator PlanetCloudsTerra, PlanetCloudsGasGiant; FUN_1405057e0 (reads DAT_14075f814); eng-gui "Clouds 0", "Clouds 1", "Clouds 2"
- Port: crates/se_core/src/procgen/mod.rs:1755-1760
- Check: FUN_1405057e0:185-193 gives a terrestrial-type body 1 + rand % PlanetCloudsTerra layers (DAT_14075f814), but the other branch (lines 173-181) hard-codes 1 or 2 layers, and PlanetCloudsGasGiant (DAT_14075f810) has no reader outside config load/save (x-ref); the port's procgen emits at most one Clouds block (procgen/mod.rs:1353 gas giants, 1756-1762 solids), and its renderer also uses a single cloud layer (terrain.rs layers.clouds), although body_params.rs:169 parses several

### 07-019 · Camera and time

Field-of-view range and animation: SE zooms down to MinFOV 0.00027778 deg (1 arcsec) and animates FOV changes over ZoomTime 0.25 s with wheel/key zoom speed ZoomAccel 0.3; port clamps at 0.001 deg and changes FOV instantly

- Status: differs, effort **S**, auditor confidence medium
- SE evidence: main-def.cfg MinFOV ZoomTime ZoomAccel
- Port: crates/se_engine/src/camera.rs:313-316,380-385
- Check: main-def.cfg:798-811 has MinFOV 0.00027778, MaxFOV 120, ZoomAccel 0.3 and ZoomTime 0.25 (read and written by the config functions FUN_14027a6a0/FUN_140280ec0); the port clamps the user FOV at 0.001 deg and changes it in one step (camera.rs:315, 381, 384); only the script Fov command is animated (scripting.rs:724)

### 07-017 · Camera and time

Camera movement modes: SE's planetarium camera has Free (Spectator), Spacecraft and Aircraft movement modes switched with keys 1-3 (splash "Free mode" / "Spacecraft mode" / "Aircraft mode", saved as MoveMode, script MoveMode), plus 4 = piloting a spacecraft; the port has the free camera and a separate ship flight mode (Enter/F, Ships window) but no Spacecraft or Aircraft camera mode and no 1-4 mode keys

- Status: missing, effort **M**, auditor confidence medium, amended by the checker
- SE evidence: FUN_1405ba8f0 MoveModeFree..MoveModeGame = 1..4; save-def.cfg MoveMode 1/2/3; eng-gui "Free mode" "Spacecraft mode" "Aircraft mode"; exe script table movemode
- Port: none
- Check: save-def.cfg documents MoveMode "1 (Spectator), 2 (Spacecraft), 3 (Aircraft), 4 (piloting spacecraft)" and the key table binds MoveModeFree..MoveModeGame to 1..4, locale "Free mode" "Spacecraft mode" "Aircraft mode"; mode 4 is ship piloting, which the port has (ship.rs flight mode, README "Flight mode"), but no camera code has Spacecraft (inertial) or Aircraft modes or the number-key switch (no aircraft/move_mode/Digit keys in crates/se_engine/src)

### 07-021 · Camera and time

Navigation script commands: SE scripts can use Horizon, Track/Untrack, MoveMode, Speed (pc/s), Fly, Turn/StopTurn, StopRotation, GotoURL and UserMoveControl/UserRotationControl/UserTimeControl (lock the player's controls in tutorials); the port parses them but skips them with a warning (StopFly/StopMotion and SpeedKm do work)

- Status: missing, effort **M**, auditor confidence high, amended by the checker
- SE evidence: exe script command table (docs/formats/se_script.md "Command table"); strings FlySpeed TurnSpeed OrbitSpeed
- Port: crates/se_engine/src/scripting.rs:568-580
- Check: The convert() in se_script.rs:482 maps "stopfly" to Op::StopMotion, which scripting.rs:445-449 runs (stops velocity, travel and flight), so StopFly works; Track/Untrack (scripting.rs:568) and Horizon, MoveMode, Speed, Fly, Turn, StopTurn, StopRotation, GotoURL and User*Control (Op::Other, scripting.rs:577) are logged and skipped, as the exe command table in docs/formats/se_script.md:164-181 lists them

### 07-020 · Camera and time

Gamepad, joystick and 3D mouse: SE reads SDL joysticks and game controllers (up to 16, dead zones, sensitivities) with default maps def_generic_controller.cfg and def_xbox360_controller.cfg (sticks fly and turn, X/A velocity, d-pad time rate, shoulders select and orbit, Start Go to, Back Center), and a SpaceNavigator; port has keyboard and mouse only

- Status: missing, effort **L**, auditor confidence high
- SE evidence: config/def_xbox360_controller.cfg, def_generic_controller.cfg; FUN_1405ba8f0 SDL_JoystickOpen / SDL_GameControllerOpen, "SpaceNavigator"
- Port: none
- Check: FUN_1405ba8f0 ("Initializing controller") calls SDL_NumJoysticks/SDL_JoystickOpen/SDL_GameControllerOpen with a 0x10 joystick cap and a "SpaceNavigator" device, and def_xbox360_controller.cfg maps sticks and triggers to CamMove/CamTurn/Ship axes with DeadZone 2000 and Sensitivity* keys; the port has no gamepad, gilrs or joystick code anywhere in crates/

### 08-030 · Interface

Journey log: SE records visited objects automatically (seJournalBrowser, journal.cfg, JournalSize 100) and lists them in the Locations dialog; port has none

- Status: missing, effort **S**, auditor confidence high
- SE evidence: rtti seJournalBrowser; main-def.cfg JournalSize; strings "journal.cfg"
- Port: none
- Check: Exe has seJournalBrowser RTTI, "Journey log" next to "Locations"/"My locations", journal.cfg, ConfigFileJournal and main-def.cfg JournalSize 100 ("maximum size of the Journey log"); port has only shipped/user Places (data.rs:35-53, README) and no journal/visited/history code in crates/.

### 08-032 · Interface

Planet vectors: SE can draw a body's rotation axis, prime meridian, orbital velocity vector and direction to parent (ShowPlanetVectors and four switches); port has none

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg ShowPlanetVectors ShowPlanetRotationAxis ShowPlanetPrimeMeridian ShowPlanetVelocityVector ShowPlanetParentVector
- Port: none
- Check: main-def.cfg:697-701 ShowPlanetVectors + ShowPlanetRotationAxis/PrimeMeridian/VelocityVector/ParentVector registered in exe (FUN_1400169e0, FUN_140016a20) with locale strings "Vectors" "Rotation axis" "Prime meridian" "Velocity vector" "Direction to parent"; port grep of crates/ for axis/meridian/vector drawing in overlay.rs, hud.rs finds nothing (rotation.rs only computes orientation)

### 08-033 · Interface

Camera velocity vector: SE can mark the direction of motion on screen (ShowVelocityVector, VelocityPointerColor); port has none

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg ShowVelocityVector; skin VelocityPointerColor
- Port: none
- Check: main-def.cfg:691 ShowVelocityVector registered by FUN_140016b60, skin colour VelocityPointerColor read in FUN_14036a560 and widget name "VelocityVector" in strings_utf16; port has no velocity marker in overlay.rs/hud.rs/ship.rs (only the selection pointer per README)

### 08-034 · Interface

Object context menu: SE offers a context menu on the selection with Parent, Satellites and Add to locations; port has none

- Status: missing, effort **S**, auditor confidence medium
- SE evidence: strings_utf16 "Context menu" "Parent" "Satellites" "Add to locations"
- Port: none
- Check: FUN_14024cb10 builds the "Context menu" window with "Parent", "Satellites", "Add to locations" entries; port has no egui context_menu/secondary-click on the selection (parent is a plain label hud.rs:263, children a collapsible list, place saving lives only in the hud save dialog hud.rs:462)

### 08-038 · Interface

GUI scaling: SE scales the interface by a GUIScale percent on top of optional Windows DPI scaling (GUIScaleDPI), set in interface settings; port follows the OS DPI implicitly via bevy_egui but has no scale percent or DPI toggle (partial)

- Status: missing, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: main-def.cfg GUIScale GUIScaleDPI; eng-gui "Interface scaling" "Use system DPI"
- Port: none
- Check: SE main-def.cfg:535-536 GUIScaleDPI/GUIScale and strings "Interface scaling", "Use system DPI" confirmed; port has no scale setting, but bevy_egui 0.43 already multiplies by the window's OS scale factor, so DPI scaling happens implicitly (not switchable)

### 08-040 · Interface

FPS meter graph: SE can show an FPS or frame-time graph with target/min/max values (fps_meter.glsl, ShowFPSMeter, ShowFrametimeMeter) and a skin-coloured FPS counter; port only has the "N fps" label in the top bar

- Status: missing, effort **S**, auditor confidence high
- SE evidence: fps_meter.glsl; main-def.cfg DebugOverlay ShowFPSMeter ShowFrametimeMeter; strings_utf16 "Tgt FPS: %i" "Max FPS"
- Port: crates/se_engine/src/hud.rs:183
- Check: SE fps_meter.glsl (graph texture, Params data head/FPS/target), DebugOverlay ShowFPSMeter/ShowFrametimeMeter and strings "Tgt FPS: %i", "Max/Min FPS", "Tgt/Max/Min time" confirmed; port has only hud.rs:183 "{:.0} fps" label and an env-var frame-time log file (system.rs:104), no graph

### 08-029 · Interface

Star browser filter options: SE's filter dialog has Sun/Planet/Moon counts, system main star and parent sun filters, atmosphere pressure classes, volatiles classes, min/max diameter, mass, density and temperature, life origin, subglacial and aerial biomes, Ignore planemo, Ignore asteroids and comets and "Add to locations"; port's filter has object kind, catalog/procedural systems, star type, class, temperature class, arid/non-arid, atmosphere yes/no, life, life type, marine/terrestrial biome, radius range and a found count

- Status: partial, effort **M**, auditor confidence high, amended by the checker
- SE evidence: eng-gui "Sun count" "System's main star" "Ignore planemo" "Ignore asteroids and comets" "Min mass"; tutorial "change the 'Sun count' to 3"
- Port: crates/se_engine/src/star_search.rs:681-718
- Check: Exe strings confirm "System's main star", "Object's parent sun", "Sun count"/"Planet count"/"Moon count", "Ignore planemo", "Ignore asteroids and comets", "Min/Max diameter/temperature/mass/density", "Add to locations"; but port star_search.rs:593-600 already shows a found count, and its filter (681-718) also has catalog/procedural systems, star type and unicellular/multicellular life type, which the gap omitted.

### 08-031 · Interface

Localization coverage: port translates only strings routed through i18n.tr; top bar ("rate", "velocity", "near:"), selection lines ("Teff", "mass ... Earth", "children"), help, Graphics, Ships and most Settings labels are fixed English or not SE keys, and object names are never taken from the *-db.cfg translations; SE translates every UI string and object name in 27 languages

- Status: partial, effort **M**, auditor confidence high
- SE evidence: data/locale/*-gui.cfg, *-db.cfg
- Port: crates/se_engine/src/hud.rs:173-183,256-321; crates/se_engine/src/graphics.rs:217
- Check: SE ships 27 *-gui.cfg (keys are the English UI strings, e.g. rus-gui.cfg "Console") and 25 *-db.cfg; port i18n.rs routes only some labels through tr (hud.rs:173-181 "rate"/"velocity"/"near:", hud.rs:256-321 "Teff"/"rotation period"/"children", help text hud.rs:477, graphics.rs:219-237 sliders, ship.rs:1203-1250 are literals) and Locale::object_name (se_core/locale.rs:184) is used only in tests while Catalog::display_name returns the raw catalog name, so object names are never translated outside the wiki heading

### 08-035 · Interface

Console window: SE has an in-game console (seConsole) with a command line and script error output ("Unknown command", "Error in line"); port has none

- Status: missing, effort **M**, auditor confidence high
- SE evidence: rtti seConsole; eng-gui "Console" "Unknown command" "Error in line"
- Port: none
- Check: rtti seConsole, eng-gui keys "Console" "Error in line" "Unknown command" "Unknown variable" (strings_utf16:2543-2548) and a bindable "Console" action in def_*controller.cfg; port only takes dev commands from CLI args/scripts (camera.rs:896 logs "unknown command" to the log) and has no in-game command-line window

### 08-037 · Interface

Music player window: SE has a player (play/pause/stop, previous/next, playlist, repeat track/playlist, in-order/random/context playing, volume, "[Playing] - " status); port only has the M and N keys

- Status: missing, effort **M**, auditor confidence high
- SE evidence: rtti seMusicPlayer; eng-gui "Music player" "Playlist" "Repeat playlist" "Context playing"
- Port: crates/se_engine/src/music.rs
- Check: SE strings_utf16:3050-3067 list the player (Play/Pause/Stop, Previous/Next track, Playlist, No repeat/Repeat track/Repeat playlist, Play in order/Play random/Context playing, Volume, "[Playing] - ") and rtti seMusicPlayer; port music.rs only has M toggle, N skip, fixed shuffle, and a volume that is saved but has no UI (set_volume is dead code)

### 08-039 · Interface

Fade transitions: SE fades overlays (orbits, markers, labels) in 0.2 s and out 0.35 s and fades+slides windows (WidgetFadeIn/Out, WidgetSlideIn/Out); port toggles overlays instantly and its egui windows only get egui's default 0.2 s fade with no slide (partial)

- Status: missing, effort **M**, auditor confidence high, amended by the checker
- SE evidence: main-def.cfg OverlayFadeIn OverlayFadeOut WidgetFadeIn WidgetFadeOut WidgetSlideIn WidgetSlideOut
- Port: crates/se_engine/src/overlay.rs:114,284
- Check: SE main-def.cfg:551-556 fade/slide durations confirmed; port overlays switch instantly (overlay.rs:114 `if !settings.orbits return`, :284 labels), but its 13 egui::Window panels already fade in over egui 0.36's default animation_time 0.2 s (Area fade_in=true, Window fade_out=true), so only the overlay fades, the slide and SE's 0.35 s fade-out timing are missing

### 08-036 · Interface

Event finder: SE's tool searches eclipses, transits, occultations and alignments (event type, precision, time step, start date, bodies table); port has none

- Status: missing, effort **L**, auditor confidence high
- SE evidence: rtti seEventFinder; eng-gui "Event finder" "Eclipse#event_finder" "Transit#event_finder"
- Port: none
- Check: SE has the window (rtti seEventFinder; strings_utf16:2353-2375 "Event finder", "Event type", "Precision", "Time step", "Start date", Alignment/Ordered alignment/Eclipse/Transit/Occultation, "Add object", "Being eclipsed"...); port has eclipse shadow rendering (eclipse.wesl) but no event search anywhere in crates/ or README

### 09-009 · Graphics and settings

Screenshot formats: SE saves screenshots as jpg (default, JpegQuality 95), png, dds, tga or tif (ScreenshotFormat, "Screenshots format"), optionally with alpha (SaveAlpha, "Save transparency"), numbered scr%05i in screenshots/ (UserScreenshotsSubDir). Port writes PNG only, named by a millisecond timestamp

- Status: partial, effort **S**, auditor confidence high
- SE evidence: main-def.cfg Export ScreenshotFormat JpegQuality SaveAlpha; strings "%sscr%05i.%S" "screenshots/" UserScreenshotsSubDir; eng-gui "Screenshots format" "Save transparency"
- Port: crates/se_engine/src/hud.rs:50-65
- Check: main-def.cfg:455-459 ScreenshotFormat "jpg" (jpg/png/dds/tga/tif), JpegQuality 95, SaveAlpha, exe strings "%sscr%05i.%S", UserScreenshotsSubDir, "Screenshots format", "Save transparency"; port F11 (hud.rs:50-65) always saves se_<millis>.png to Pictures/SpaceEngine-rs, with no format, quality, alpha or numbering option (dev `shot` command is test-only)

### 09-007 · Graphics and settings

Graphics presets apply only 7 keys: SE loads the whole preset-*.cfg, so a preset also sets VideoMemorySize, TextureCompress, StarPointMethod, PlanetPointMethod, PlanetDetailTextures, PlanetShineMode, AuroraQuality, ShipEngineQuality, WarpPrecision, BlackHoleQuality, ShipWarpQuality, the TextureRendering resolutions, envmap sizes, upscale mode, MSAALevelImp and the loader time limits; on a hardware or version change SE also picks the preset from VRAM (<=2 GB Potato, <=3 GB Low, <=4 GB Medium, else High). Port maps only MSAA, FXAA, anisotropy, LandLOD, LandTargetRes, MaxTilesPerFrame and VolumResolution, uses High on first run instead of a VRAM-based choice (it then restores the saved preset), and runs twice MaxTilesPerFrame

- Status: partial, effort **M**, auditor confidence high, amended by the checker
- SE evidence: config/preset-*.cfg; FUN_140288db0; FUN_14030b0e0 ("Hardware or SE version (build number) change detected - loading the optimal config"); FUN_14022dfb0
- Port: crates/se_engine/src/graphics.rs:4-8,44-70,93,164
- Check: preset-*.cfg (103 lines each) sets VideoMemorySize, TextureCompress, Star/PlanetPointMethod, PlanetShineMode, AuroraQuality, warp/black-hole qualities, TextureRendering, MSAALevelImp and Loader keys, FUN_140288db0 picks the preset-*.cfg file, and FUN_14030b0e0 picks a preset from VRAM (<=2 GB 0, <=3 GB 1, <=4 GB 2, else 3) on a hardware/build change; Quality::from_cfg reads only 7 keys and graphics.rs:164 doubles MaxTilesPerFrame, but the port does not "always start on High": settings.rs load_graphics restores the saved preset and values, so High is only the first-run default

### 09-008 · Graphics and settings

Tone mapping operator choice: SE lists every data/shaders/*_tone.glh as a "Tone mapping" choice (ToneMappingShader: ACES_Fitted, AgX, AgX_Punchy, Bruneton, Exposure, Filmic, Filmic_Hejl, Rec_709, Reinhard, Reinhard_SE, sRGB; "seRenderer::Init(): No tone mapping shader were found"). Port is fixed to AgX, with ACES only through the SE_TONEMAP env var

- Status: missing, effort **M**, auditor confidence high
- SE evidence: hdrt_*_tone.glh (11 files); hdr_tone.glsl; main-def.cfg ToneMappingShader; eng-gui "Tone mapping"; FUN_1403238d0
- Port: crates/se_engine/src/camera.rs:172-177
- Check: re/se_shaders has 11 hdrt_*_tone.glh files plus hdr_tone.glsl, main-def.cfg:221 ToneMappingShader "AgX", FUN_1403238d0 scans for "_tone.glh" and logs "No tone mapping shader were found", exe has "Tone mapping" UI string; port camera.rs:172-177 hard-codes Tonemapping::AgX with ACES only via env SE_TONEMAP (README:43), no other tonemapper selection in crates/

### 09-010 · Graphics and settings

High-resolution screenshot tool: SE renders screenshots larger than the window as tiles (seScreenshotTool: Target resolution, Tile resolution / count, Max dimensions, Max size in memory, Scale stars, its own landscape LOD ScreenshotLandLOD/ScreenshotTileRes, "Wait loading completeness") into screenshots-hd/. Port has none

- Status: missing, effort **M**, auditor confidence high
- SE evidence: rtti seScreenshotTool; strings "Starting high-resolution screenshot rendering: %i x %i, %i x %i tiles of %i x %i" "screenshots-hd/" ScreenshotLandLOD ScreenshotTileRes UserScreenshotsHDSubDir; eng-gui "High resolution screenshot" "Target resolution" "Scale stars" "Tile resolution"
- Port: none
- Check: rtti seScreenshotTool, exe strings "Starting high-resolution screenshot rendering: %i x %i, %i x %i tiles...", "screenshots-hd/", ScreenshotLandLOD, ScreenshotTileRes, UserScreenshotsHDSubDir, and UTF-16 "High resolution screenshot", "Target resolution", "Scale stars", "Tile resolution", "Max size in memory", "Wait loading"; no tiled or above-window-size capture anywhere in crates/ (only window-size Screenshot::primary_window)

### 09-011 · Graphics and settings

Skybox / panorama export: SE exports the sky around the camera as a 6-face cubemap or a cylindrical panorama (seExportSkyboxDialog: Resolution, Image format SkyboxFormat tga/jpg/png/dds/tif, Pixel format 8-bit / 16-bit / 32-bit float, Post-process or "Color correction (no HDR!)", Save transparency). Port has none

- Status: missing, effort **M**, auditor confidence high
- SE evidence: rtti seExportSkyboxDialog; main-def.cfg Export SkyboxFormat; eng-gui "Export skybox" "Cubemap" "Cylindrical" "Pixel format" "32-bit (floating-point)" "Post-process" "Color correction (no HDR!)"; hdr_copy_cube.glsl
- Port: none
- Check: SE has seExportSkyboxDialog (rtti_classes.txt:145), Export SkyboxFormat "tga" (main-def.cfg:456), hdr_copy_cube.glsl samplerCube, and locale keys "Export skybox" "Cylindrical" "32-bit (floating-point)" "Color correction (no HDR!)" (in the *-gui.cfg files; this install's eng-gui.cfg is a 5 KB stub); port grep for skybox/cubemap/panorama/equirect finds only galaxy-model loader code (galaxy_model.rs CubeMap type), no export path

### 09-015 · Graphics and settings

Memory limits and caches: SE caps VRAM and RAM (VideoMemoryStrategy, VideoMemorySize 3072 MB, VideoMemoryReserved, RAMSizeMax 8192 MB / RAMSizeMaxPercentage 85, logs Total/Available/Max SE RAM and available VRAM) and sizes its caches from config (CacheSizeSolarSystems 64 planetary systems evicted LRU, CacheSizeLandNode 10000, CacheSizeStarNode 50000, CacheSizeGalNode 10000, CacheSizeLandTextures, galaxy/nebula/atmosphere model caches). Port has no memory budget or VRAM/RAM logging and hard-coded, non-configurable limits (terrain.rs:50 2500 nodes, procgalaxies.rs:47 1400 nodes, procsystems.rs:58 6 in-galaxy cluster/nebula systems), while generated planetary systems are added to the catalog forever (system.rs:384-394), so memory grows without bound over a long session

- Status: missing, effort **M**, auditor confidence medium, amended by the checker
- SE evidence: main-def.cfg Memory 6-26; FUN_14030b0e0 ("Max SE RAM: %d Mb", "seMemoryManager::UpdateAvailableVRAM()"); strings "Available VRAM: %1.2f Mb"
- Port: crates/se_engine/src/procsystems.rs:50-58; crates/se_engine/src/terrain.rs:50; crates/se_engine/src/procgalaxies.rs:47
- Check: SE side checks out (main-def.cfg Memory 6-26 VideoMemoryStrategy/VideoMemorySize 3072/RAMSizeMax 8192/RAMSizeMaxPercentage 85/CacheSize* keys; FUN_14030b0e0 prints "Total/Available/Max SE RAM" and UpdateAvailableVRAM "Not enough available video memory"), but the port details are wrong: procsystems.rs:58 MAX_CACHE 6 caches the in-galaxy cluster/nebula systems, not planetary systems, and generated planetary systems are not regenerated on revisit but appended to the catalog permanently (system.rs:384-394 materialize_star -> cat.load_sc), so they are never freed

### 09-016 · Graphics and settings

Settings persistence: SE keeps every setting in config/main-user.cfg as overrides of main-def.cfg (created on first run, fully updated after a version change, DetectVersionChange), including window state, photo mode, post-effects, projection, export and loader keys. Port saves its own subset (star, overlay, FOV, music, language, the 7 preset keys, lens flares) to %APPDATA%/SpaceEngine-rs/settings.cfg and reads main-def.cfg only for lens flares and the procedural generators, so other main-def defaults are hard-coded copies

- Status: partial, effort **M**, auditor confidence high
- SE evidence: strings "main-user.cfg" "Creating user config file \"%s\"" "Performing full update of the config file \"%s\"" DetectVersionChange
- Port: crates/se_engine/src/settings.rs:43-136
- Check: SE strings main-user.cfg, "Creating user config file", "Performing full update of the config file", DetectVersionChange exist (strings_ascii.txt:26306-26389); port settings.rs Snapshot persists only star/overlay/FOV/music/language/graphics-preset/lens-flare keys to %APPDATA%/SpaceEngine-rs/settings.cfg and only lensflare.rs:233, procgalaxies.rs:165 (plus graphics.rs:141 preset-*.cfg) read SE config files; Settings-window toggles such as ShowProcGalaxies are not saved

### 09-012 · Graphics and settings

Video capture: SE records video through FFmpeg (seVideoRecorder / seAsyncVideoRecorder: encoders from config/ffmpeg.cfg, NVENC/AMF/QSV hardware or libsvtav1/libaom-av1, pixel formats YUV420P..P010, quality and bitrate presets, VideoFrameRate 60, streaming (real-time) or offline (fixed-FPS, Frame time mode with RealTime/RealTimeFPS) capture, frame-rate capping, with or without GUI, auto-hiding recording tools) or as numbered screenshot frames (VideoFrameFormat). Port has none

- Status: missing, effort **L**, auditor confidence high
- SE evidence: config/ffmpeg.cfg; main-def.cfg Export 453-475; strings "seVideoRecorder::StartCapture() Capture started with encoder"; eng-gui "Video capture" "Start recording" "Record as video file" "Record as screenshots" "Capture mode" "Streaming (real time)" "Offline rendering (fixed FPS)" "Framerate capping" "Frame timing mode"; def_generic_controller.cfg VideoCaptureDialog VideoRecording
- Port: none
- Check: config/ffmpeg.cfg lists av1_nvenc/amf/qsv/libsvtav1/libaom-av1 with streaming/offline options, main-def.cfg Export 453-475 has VideoCaptureInAVI/VideoFrameRate 60/VideoEncoder/VideoFpsCapMethod/RealTime, strings_ascii.txt:24977-24997 seVideoRecorder/seAsyncVideoRecorder StartCapture, def_generic_controller.cfg:162-163 VideoCaptureDialog/VideoRecording; port has only single screenshots (README `shot FILE`, `--screenshot`) and no ffmpeg/video/frame-sequence code

### 09-013 · Graphics and settings

Projection modes: SE renders a cubemap and resamples it into fish-eye (proj_fisheye.glsl: FisheyeFOV 180, circular crop, zoom, offset, off-axis warp, flips, dome GUI angle and optional interface distortion) or 360-degree cylindrical (proj_cylinder.glsl) projections, or shows the cube faces as a mosaic (ProjectionMode 1, CubemapFormat, ShowCubempEdges, FitCubemapToScreen), with their own cube-face / GUI / lens-flare resolutions. Port has perspective only

- Status: missing, effort **L**, auditor confidence high
- SE evidence: proj_fisheye.glsl:20-50; proj_cylinder.glsl:18-38; main-def.cfg Projection ProjectionMode Fisheye* CubeFaceResolution GUIResolution LensFlareResolution DistortInterface; eng-gui "Perspective" "Fish eye" "Cylindrical" "Configure projection" "Lens angle" "Circle crop" "X warp" "Identify faces" "Fit to screen"
- Port: crates/se_engine/src/camera.rs:188-192
- Check: main-def.cfg Projection has ProjectionMode 0-3 (perspective/cubic/fish eye/cylinder), Fisheye FOV/Crop/Zoom/Offset/Warp/Flip, CubeFaceResolution/GUIResolution/LensFlareResolution; proj_fisheye.glsl samples a samplerCube with Param(LensAngle,CutOff)/Offset/Warp and a GUI variant, proj_cylinder.glsl resamples the cube to a 360x180 lat-long view; port camera.rs:188-192 spawns a single Projection::Perspective and grep finds no fisheye/cubic/cylinder projection

### 09-014 · Graphics and settings

Render resolution and upscaling: SE renders solid objects at SolidResolution (0.5 default, 0.7-1.0 in the presets, capped by SolidTargetRes 1080) and volumetrics at VolumResolution 0.35, then upscales them (SolidUpscaleMode/VolumUpscaleMode: nearest, bilinear, bicubic, lanczos, each plain/sharp/CAS, or checkerboard upscale_checkerboard.glsl), optionally spread over frames (SolidTemporalUpscale, combine_temporal.glsl, "Fast temporal rendering"), FXAA'd before upscaling (SolidFXAA) and bilateral-filtered (VolumBilateral, bilateral.glsl), and sharpens the final frame with AMD FidelityFX CAS at PostEffects Sharpness 1 (hdr_final.glsl SHARP). Port renders everything at window resolution with no sharpening, and uses VolumResolution as a nebula ray-march step count

- Status: missing, effort **L**, auditor confidence medium
- SE evidence: upscale_bicubic.glsl; upscale_checkerboard.glsl; combine_temporal.glsl; bilateral.glsl; hdr_final.glsl:38-170 textureFastCAS; main-def.cfg TextureRendering 316-349, PostEffects Sharpness; eng-gui "Sharpness" "Fast temporal rendering" "Volumetric objects resolution" "While moving" "While stationary"
- Port: crates/se_engine/src/graphics.rs:66-67; crates/se_engine/src/nebula_raymarch.rs:275
- Check: main-def.cfg TextureRendering 316-349 has SolidTargetRes 1080, SolidResolution 0.5, VolumResolution 0.35, Solid/VolumUpscaleMode 8 (bicubic CAS), SolidTemporalUpscale, SolidFXAA, VolumBilateral, PostEffects Sharpness 1 (line 200), and hdr_final.glsl:39-175 defines textureCAS/textureFastCAS under SHARP; port graphics.rs:4-8 says render scale is "kept only for display", there is no render-scale, upscale, TAA or ContrastAdaptiveSharpening anywhere in crates/, and nebula_raymarch.rs:275-287 spends volume_resolution on ray-march step count

### 10-019 · Scripts, audio, spacecraft

Engine control model: SE commands main, retro and hover engines as separate levels (ShipMain/Retro/HoverEnginePlus/Minus) plus thrusters (ShipMove*), and has settings for Turn by keys speed, Roll by keys speed and Turn by mouse speed. Port uses one throttle from -1 to 1 (below zero is retro), fires hover only through the R/F RCS keys, and has fixed turn rates

- Status: differs, effort **S**, auditor confidence medium
- SE evidence: exe actions ShipMainEnginePlus .. ShipHoverEngineMinus, ShipMoveForward ..; gui "Main engines +" "Hover engines -" "Turn by keys speed" "Roll by keys speed" "Turn by mouse speed"
- Port: crates/se_engine/src/ship.rs:540-565
- Check: exe utf16 has ShipMain/Retro/HoverEnginePlus/Minus, ShipMoveForward..Down, "Main engines +", "Hover engines -", "Turn by keys speed", "Roll by keys speed", "Turn by mouse speed"; port has one throttle -1..1 with retro below zero (ship.rs:117,540-555,810), hover only via R/F RCS input (ship.rs:809-813), and fixed MAX_TURN_RATE/MOUSE_TURN_GAIN constants with no speed settings.

### 10-026 · Scripts, audio, spacecraft

Ship brightness keys: SE scales ship models by the design's Exposure (1.5-3), Albedo and Color and by hdrShipModelBright/realShipModelBright; port parses Exposure and Albedo but never uses them, does not parse Color, and has no model-brightness setting, so ship brightness differs from SE

- Status: missing, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: ShipsModels.pak *.sss Exposure/Albedo/Color (29); main-def.cfg Lighting hdrShipModelBright, realShipModelBright; ship.glsl Params.w exposure, DiffColorScale
- Port: crates/se_core/src/ship.rs:174-175 (parsed only)
- Check: All 29 stock .sss set Albedo, Exposure (25x 2, 3x 3, 1x 1.5) and Color, and ship.glsl:416 multiplies the final colour by Params.w (exposure) with DiffColorScale at :221, while the port's ShipDesign stores albedo/exposure but no engine code reads them (ship.wesl has fixed tint, ambient 0.07 and a clamped star intensity, ship.rs:642,1113) and Color is not parsed at all; hdr/realShipModelBright (main-def.cfg:270,292) are absent.

### 10-028 · Scripts, audio, spacecraft

Script music and sound commands: PlayMusic "file.ogg" (used by 2 tutorials), PauseMusic, ResumeMusic, FadeOut, FadeIn (cross-fading over MixDurationScript 1 s) and SetAudioDevice. Port skips them

- Status: missing, effort **S**, auditor confidence high
- SE evidence: Scripts.pak tutorials PlayMusic; exe command table playmusic, pausemusic, resumemusic, fadeout, fadein, setaudiodevice; main-def.cfg MixDurationScript
- Port: crates/se_engine/src/scripting.rs:577-580
- Check: Exe command table holds playmusic, pausemusic, resumemusic, fadeout, fadein, setaudiodevice and main-def.cfg:434 MixDurationScript; tut-05-univ-map.se:297 and tut-07 use PlayMusic, but se_script.rs has no music ops so they fall to Op::Other and scripting.rs:577-580 logs "not supported, skipping" even though the port has a music player (music.rs).

### 10-020 · Scripts, audio, spacecraft

Ship camera views: SE has Auto view, Behind view and Cockpit view ("Switch view"), look back/left/right/up/down (ShipLookBack..ShipLookDown), Center on ship (ShipCenter) and ShipViewOrbit, and keeps the cockpit camera fixed ("Can't move camera in cockpit view"). Port cycles the Behind presets, then Cockpit, then a free orbit view with V, and has no look directions or center-on-ship

- Status: partial, effort **M**, auditor confidence medium
- SE evidence: exe actions ShipLook*, ShipCenter, ShipViewOrbit; gui "Auto view" "Cockpit view" "Behind view" "Center on ship"; *.sss Camera "Auto"/"Behind"/"Cockpit"
- Port: crates/se_engine/src/ship.rs:1018-1062; crates/se_core/src/ship.rs:254-272
- Check: exe utf16 has ShipLookBack..ShipLookDown, ShipViewOrbit, ShipCenter, "Switch view", "Auto view", "Cockpit view", "Behind view", "Center on ship", "Can't move camera in cockpit view", and .sss files carry Camera "Auto"/"Behind"; port parses CameraKind::Auto but never uses it (chase_cameras takes Behind then Cockpit only, se_core ship.rs:268-272), V cycles those then a free orbit (ship.rs:576, 1029-1041), cockpit camera is scaled by wheel zoom, and there are no look directions or center-on-ship.

### 10-021 · Scripts, audio, spacecraft

Hyperdrive: SE's gui has a hyperspace mode separate from the warp drive: Hyperjump, Hyperjump here, Hyperspace/Normal space, Stop hyperdrive, Preparing/Correcting jump vector, and "Field is unstable, turn to the target!". It also has a warp order (ShipWarpOrder). Port treats Hyperdrive designs exactly like Warpdrive (10^boost velocity)

- Status: missing, effort **M**, auditor confidence low
- SE evidence: gui "Hyperjump" "Hyperspace" "Normal space" "Stop hyperdrive" "Field is unstable, turn to the target!"; exe ShipWarpOrder; *.sss Hyperdrive true (6 designs)
- Port: crates/se_core/src/ship.rs:281
- Check: Exe UTF-16 strings list a separate hyperdrive UI ("Hyperjump", "Hyperjump here", "Hyperspace"/"Normal space", "Stop hyperdrive" next to "Stop warp drive", "Preparing/Correcting jump vector", "Field is unstable, turn to the target!") and ASCII ShipWarpOrder beside CheatsMenu keys; port only has ShipDesign::can_warp() = warpdrive // hyperdrive (se_core/src/ship.rs:281) feeding the same 10^boost warp in se_engine/src/ship.rs:580,861, no hyperjump code anywhere

### 10-022 · Scripts, audio, spacecraft

Cheats and survivability: SE has a Cheats dialog (CheatsMenu) with Unlimited thruster acceleration (UFO), Unlimited velocity (FTL), Disable aerodynamics, Disable gravity, Disable collision detection, invulnerability (God) and TARDIS, and shows a "Cheater!" mark. With God off a crash destroys the ship ("Destroying spacecraft"). Port's Ships window has only Gravity and Collisions checkboxes; a crash is a notice and speed is always capped at c outside warp

- Status: partial, effort **M**, auditor confidence high
- SE evidence: main-def.cfg Physics CheatsMenu, UFO, FTL, God, Aero, Gravity, Collisions, TARDIS; exe rtti seShipsManagerCheatsDialog; gui "Unlimited thruster acceleration" "Cheater!"
- Port: crates/se_engine/src/ship.rs:1258-1262,904-905
- Check: main-def.cfg Physics 442-449 has CheatsMenu/UFO/FTL/God(true)/Aero/Gravity/Collisions/TARDIS, rtti seShipsManagerCheatsDialog, UTF-16 "Cheats", "Cheater!", "Unlimited thruster acceleration", "Unlimited velocity", "Time travel", "Invulnerability", "Disable aerodynamics", and ASCII "Destroying spacecraft"; port Ships window has only Gravity/Collisions/Rotation damping checkboxes (ship.rs:1258-1260), clamps sub-warp speed to c (ship.rs:904-905) and a crash is only a notice (ship.rs:933-934)

### 10-023 · Scripts, audio, spacecraft

Catalog spacecraft (static ships): Catalogs.pak planets/ApolloLandingSites.sc places six Apollo LMs (Model spacecraft/real/ApolloEagle.sss) on the Moon with FixedPosPolar (lon lat dist), TidalLocked, Obliquity, EqAscendNode and RotationOffset; SE builds them as static ships ("Initializing static ships", LoadStaticShip). Port ignores FixedPosPolar and the orientation keys: "LM Eagle" sits at the Moon's centre, so the landers never show on the surface

- Status: missing, effort **M**, auditor confidence high, amended by the checker
- SE evidence: Catalogs.pak planets/ApolloLandingSites.sc; exe "Initializing static ships", seSpacecraft::LoadStaticShip
- Port: crates/se_core/src/catalog.rs (no FixedPos); .audit/work/10/lm.png
- Check: ApolloLandingSites.sc has 6 Spacecraft blocks with Model ApolloEagle.sss, ParentBody Moon, TidalLocked, Obliquity, EqAscendNode, RotationOffset, FixedPosPolar (no Type key in the file; "StaticReal" is only an exe string), exe "Initializing static ships" and LoadStaticShip; no FixedPos handling anywhere in crates/, and .audit/work/10/lm.png shows "LM Eagle" at 1740.282 km while the camera is 1.4 km above the Moon (placed at the Moon's centre)

### 10-024 · Scripts, audio, spacecraft

Ship trajectory: SE can draw the ship's predicted orbit ("View ship trajectory", ShowOrbitsShips, ShipSelectedOrbitColor) and shows ships with labels and markers (ShowLabelsShips, ShowMarkersShips, InfoSpacecraftColor). Port draws no trajectory, label or marker for the ship

- Status: missing, effort **M**, auditor confidence medium
- SE evidence: gui "View ship trajectory"; main-def.cfg ShowOrbitsShips, ShowLabelsShips, ShowMarkersShips; exe ShipSelectedOrbitColor
- Port: none
- Check: main-def.cfg ShowOrbitsShips/ShowLabelsShips/ShowMarkersShips (lines 713,723,738, all true), exe InfoSpacecraftColor, ShipSelectedOrbitColor and UTF-16 "View ship trajectory"; port overlay.rs builds markers/labels/orbits only from catalog bodies and stars, and ship.rs has no label, marker or trajectory code (its "shiporbit" is a camera mode)

### 10-025 · Scripts, audio, spacecraft

Ship lighting: SE lights hulls with up to 4 light sources (MAX_LIGHTS), eclipse shadows from bodies (ECL, EclipseCasters, 8 per light) and the ambient level of AmbientParams. Port lights hulls with the brightest star of the system only, plus a fixed fill, with no shadows from planets or moons

- Status: partial, effort **M**, auditor confidence high
- SE evidence: re/se_shaders/ship.glsl:20-21,277-315
- Port: crates/se_engine/assets/shaders/ship.wesl:58-67; crates/se_engine/src/ship.rs:1100-1115
- Check: ship.glsl:20-21 MAX_LIGHTS 4 / MAX_ECLIPSES 8, 283-315 loops over NLights with Lambert/Hapke mix (AmbientParams.w) and ECL EclipseShadowFar per light; port ship.wesl:58-67 uses one sun_dir + fixed hemispheric fill, set in ship.rs:1099-1120 from the single brightest star, no eclipse shadows

### 10-029 · Scripts, audio, spacecraft

Script camera commands: Track/Untrack (port logs "not supported"), Horizon, Fly, Turn/StopTurn, StopRotation, Speed (pc/s), MoveMode, Bind and GotoURL (se:// location) are not implemented (StopFly is only aliased to StopMotion); Goto ignores its flight profile (AccelTime, DriftTime, DecelTime, AccelPower, DecelPower); CheckVersion does not stop scripts made for another version

- Status: missing, effort **M**, auditor confidence high, amended by the checker
- SE evidence: exe command table track, untrack, horizon, fly, stopfly, turn, stopturn, stoprotation, speed, movemode, bind, gotourl, checkversion; exe AccelTime/DriftTime strings; gui "This script was designed for a different version"
- Port: crates/se_engine/src/scripting.rs:384,568-580
- Check: Exe command table (raw .rdata at 0x14075f9xx) lists horizon, track, untrack, gotourl, movemode, speed, fly, stopfly, turn, stopturn, stoprotation and bind, and FUN_140563f30 reads Goto keys AccelTime/DriftTime/DecelTime/AccelPower/DecelPower; port maps Track/Untrack to a "not supported" warning (scripting.rs:568), sends the rest to Op::Other, parse_goto (se_script.rs:584) ignores the profile keys and Op::CheckVersion is a no-op, but StopFly is accepted as an alias of StopMotion (se_script.rs:482).

### 10-018 · Scripts, audio, spacecraft

Docking: 27 DockingPort blocks in 5 module cfgs (Type "Passenger", CorrPos/CorrSize/CorrRot, ShipPos/ShipRot). SE has a Docking HUD, a seSpacecraftDockingDialog listing Docking ports, Docked ships and Free docks, readouts for Distance to dock and Angle with dock, Docking/Undocking with "Docked with", and saved DockedShipID/DockedPort. Port does not parse docking ports and has no docking

- Status: missing, effort **L**, auditor confidence high, amended by the checker
- SE evidence: ShipsModels.pak *.cfg DockingPort (27); exe rtti seSpacecraftDockingDialog; exe DockedShipID, DockedPort, "Spacecraft docked to unknown ship"; gui "Docking HUD" "Free docks" "Angle with dock"
- Port: none
- Check: DockingPort blocks exist (Type "Passenger", CorrPos/CorrRot/CorrSize, ShipPos/ShipRot) but there are 27 blocks in 5 cfgs (shuttle_deck_front/back 12 each, AmS-S, Shuttle, Skylone), not 27 cfgs; exe has seSpacecraftDockingDialog RTTI, DockedShipID/DockedPort, "Docking HUD", "Docking ports", "Docked ships", "Free docks", "Distance to dock", "Angle with dock", "Docked with", "Docking"/"Undocking"; port has no "dock" anywhere in crates/ and README lists docking as not implemented.

### 10-027 · Scripts, audio, spacecraft

PBR ship materials: with EnableShipPBR true (default), materials with MaterialPBR/PbrMap/RoughnessScale/RoughnessConst/MetallnessScale/MetallnessConst are drawn by ship_pbr.glsl, which has image-based lighting from environment cubemaps (EnvMapSpec, EnvMapDiff, IBL_*), AO and normal anti-aliasing. EnvMap/Reflections materials get cube-map reflections. Port has no PBR, IBL or reflections

- Status: missing, effort **L**, auditor confidence high
- SE evidence: re/se_shaders/ship_pbr.glsl:53-55,241-254,1830-1872; ship.glsl ENVMAP; main-def.cfg Engine EnableShipPBR; ShipsTex.pak *.sml MaterialPBR, EnvMap, Reflections
- Port: none
- Check: ship_pbr.glsl:53-55 binds EnvMap/EnvMapSpec/EnvMapDiff with IBL_* defines at :242-244, EnableShipPBR true at main-def.cfg:1061, and ShipsTex.pak uses MaterialPBR/PbrMap (real/ApolloEagle.sml), EnvMap (modules/SHW.sml x3) and Reflections (default/Skylone.sml); port parse_ship_materials (se_core/src/ship.rs:507) reads only "Material" blocks and ship.wesl is Lambert+emissive with no PBR, IBL or cubemap reflection.


## Visibility 1: obscure

### 01-025 · Stars

black hole settings: Black hole quality (BlackHoleQuality 0-4), Black hole update interval and BlackHoleWarpRadius are missing; the port has a fixed quality and SE_NO_BLACKHOLES only

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg WarpRendering BlackHoleQuality, BlackHoleWarpRadius; exe "Black hole quality", "Black hole update interval"
- Port: crates/se_engine/src/blackholes.rs
- Check: main-def.cfg:187/189 has BlackHoleQuality 3 (0-4) and BlackHoleWarpRadius 100 and strings_utf16.txt:2826/2828 has "Black hole quality"/"Black hole update interval", while the port's blackholes.rs uses a fixed MAX_STEPS 220 in its lens shader and a hard-coded influence radius (blackholes.rs:250-262); neither settings.rs nor config.rs reads any of these, and SE_NO_BLACKHOLES (README:40) is the only switch.

### 01-026 · Stars

catalog star flags ForceAppMagn (use the given system AppMagn instead of the sum of its components) and Hidden (e.g. "Menu Star") are ignored

- Status: missing, effort **S**, auditor confidence high
- SE evidence: Catalogs.pak stars/Stars-bin.sc:831 ForceAppMagn, stars/SpaceEngine.sc Hidden true
- Port: crates/se_core/src/catalog.rs:308-399
- Check: Catalogs.pak has ForceAppMagn true on three bright barycenters (Stars-bin.sc:831 BET Sco, :1145 SIG Ori, :2177 ETA Car; exe string ForceAppMagn) and Hidden true on "Menu Star" (SpaceEngine.sc:14; exe string Hidden), but nothing in crates/ matches ForceAppMagn, Hidden or "Menu Star"; the port's starfield skips barycenters and draws each component (stars.rs:285), so the forced magnitude is never used and Menu Star is drawn like any other star.

### 01-027 · Stars

planet points: SE moves a point-like planet's sprite towards its sunlit crescent by phase*radius (OffsetToCrescent); the port pulls the sprite in front of the body along the view ray (like SE) but leaves out the sunward crescent shift

- Status: missing, effort **S**, auditor confidence high, amended by the checker
- SE evidence: star_point.glsl:53-68; star_sprite.glsl:57-76
- Port: crates/se_engine/src/system.rs:660-668
- Check: star_point.glsl:53-68 and star_sprite.glsl:57-76 shift a planet's point sprite by phase*radius towards the sun; the port (system.rs:660-668) already pulls the sprite in front of the body along the view ray by 2r/dist (SE's -2*radius/dist term), but it has no sunward term, so the gap is only the crescent shift, which shows only while the sprite fades out between 0.5 and 1.5 px.

### 02-017 · Star clusters

Cluster debug overlay: SE's ShowClusOctree draws the galaxy's cluster octree and the debug stats count visible clusters ("Cluster octree:", "Number of clusters"); port has neither

- Status: missing, effort **M**, auditor confidence high
- SE evidence: main-def.cfg ShowClusOctree, exe "Show Cluster octree", FUN_140331f60, FUN_140499ce0
- Port: none
- Check: SE: main-def.cfg:786 ShowClusOctree (read by FUN_14033e950), strings "Show Cluster octree", "Cluster octree: " (FUN_140242790) and "Number of clusters" (FUN_14038a450), FUN_140499ce0 counts visible clusters with the Cat/Proc filters; port has no cluster octree at all (flat member mesh, clusters.rs) and its only deep-sky stats are "galaxy nodes, galaxies loaded" (procsystems.rs:173), no cluster count or octree drawing

### 04-021 · Planet surfaces

Terrain LOD and loading options: SE's LandLOD runs -2..+2 with IncPlanetLOD/DecPlanetLOD hotkeys, PlanetMaxLevel 24 (debug menu "Terrain max level") and PlanetReverseLoading (Graphics "Fast landscape loading", highest nodes first); the port's landscape LOD slider runs -3..+1 with no hotkeys, its depth is fixed at MAX_GEOM_LEVEL 20, and there is no reverse-loading option

- Status: partial, effort **S**, auditor confidence high, amended by the checker
- SE evidence: main-def.cfg LandLOD, PlanetMaxLevel, PlanetReverseLoading; locale "Terrain max level", "Fast landscape loading", "Increase landscape LOD"; def_generic_controller.cfg IncPlanetLOD
- Port: crates/se_engine/src/graphics.rs:235; crates/se_engine/src/terrain.rs:46
- Check: LandLOD -2..+2 (main-def.cfg:87, presets), PlanetMaxLevel 24, PlanetReverseLoading, IncPlanetLOD/DecPlanetLOD and the "Fast landscape loading" Graphics string are confirmed while the port's slider is -3..+1 (graphics.rs:235), no hotkey changes land_lod and depth is the constant MAX_GEOM_LEVEL 20 (terrain.rs:46); but "Wait loading completeness" sits among the planet-texture export dialog strings (Cubemap, Elevation map, Generating, Saving files), not terrain options, and "Terrain max level" is a debug-menu entry.

### 05-023 · Atmospheres, rings, comets

Atmosphere table generation: SE generates scattering tables on the GPU (ag_*.glsl, seAtmosphereModelCache RecreateModel/Precompute/SaveBinary) for any atmospheres.cfg model whose .atm file is missing, honouring AtmoHalfPrecision (FP16 files), AtmoAnalyticTransm and AtmoFixHorizon when the cache is removed, and caches them (CacheSizeAtmoModels); the port only loads pre-shipped .atm files, so add-on atmosphere models without an .atm and these options have no effect (stock install unaffected: all 13 active models ship as .atm)

- Status: missing, effort **L**, auditor confidence high, amended by the checker
- SE evidence: exe "Generating atmosphere model \"%s\"" FUN_14044a760; strings seAtmosphereModelCache::Precompute; ag_*.glsl; main-def.cfg AtmoHalfPrecision AtmoAnalyticTransm AtmoFixHorizon
- Port: crates/se_engine/src/atmosphere.rs:262-278
- Check: FUN_14044ad90 loads "<model>.atm" via LoadBinary FUN_14044b650 if the file exists and otherwise (when precompute is enabled) queues task 6 whose handler RecreateModel FUN_14044a760 reads the model from atmospheres.cfg, logs "Generating atmosphere model" and runs the ag_*.glsl precompute and SaveBinary FUN_14044b060; the stock Atmospheres.pak ships .atm files for all 13 uncommented atmospheres.cfg models, and main-def.cfg says AtmoHalfPrecision/AtmoAnalyticTransm/AtmoFixHorizon apply only after "remove cache", so changing them does not by itself regenerate; the port (atmosphere.rs:262-278) only loads shipped .atm files and never reads atmospheres.cfg (no RayleighBeta/MieScaBeta hit in crates/)

### 06-019 · Procedural generation

Moon names only partly match: SE numbers moons in three series: plain "<planet>.<n>" (type 5), "<planet>.D<n>" (other types) and "<planet>.S<n>" (flag 0x2000; seen in SE's places-def as "2.S24", "1.S7"); the port uses only ".<n>" for Moon and ".S<n>" for DwarfMoon, has no ".D<n>" series, and has no lettered binary-planet components ("B5b", "A6b", "3a"); the type-5 and flag conditions are not yet decoded

- Status: differs, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: FUN_1404fdc90:943-1010 (string "D" at 0x1406c5280); save/places-def.cfg:468
- Port: crates/se_core/src/procgen/mod.rs (gen_moons naming, docs/procgen.md "Naming")
- Check: FUN_1404fdc90:943-1021 (and 560-600) has three moon-number forms: plain "%d" (DAT_1406bd48c) for body type 5, "D"+n (L"D" at 0x1406c5280) for other types, and "S"+n (L"S" at 0x140687dec) when flag bit 13 of +0x11c0 is set; SE's own save/places-def.cfg holds "RS 0-1-3-1102-3821-2-11-619 2.S24" and "...6081-18 1.S7", so the port's ".S<n>" (procgen/mod.rs:2091-2099) is a real SE form, not an invention; what is missing is the "D" counter and the lettered binary-planet components (port has no binary planets)

### 07-022 · Camera and time

Timing and loader modes: SE switches between real-time and fixed frame-time simulation (TimingMode, Shift+\, "Real time mode" / "Frame time mode", used for video capture) and asynchronous loading (LoaderMode, "Asynchronous loading mode"); port always advances by real frame time

- Status: missing, effort **S**, auditor confidence medium
- SE evidence: FUN_1405ba8f0 TimingMode = Shift+\; eng-gui "Real time mode" "Frame time mode" "Asynchronous loading mode"
- Port: crates/se_engine/src/universe.rs:131-134
- Check: The exe has the UTF-16 strings "Real time mode"/"Frame time mode" and "Immediate/Interleaved/Asynchronous loading mode" (strings_utf16.txt:2998-3003; they are in the other *-gui.cfg locales, not in eng-gui.cfg), TimingMode/LoaderMode are bindable actions (controller cfgs, default Shift+\ in the auditor's default-key table) and main-def.cfg:477 has LoaderMode 0/1/2; the port's advance_time (universe.rs:131-134) always uses the real frame delta and has no fixed-frame-time or loader-mode switch

### 08-041 · Interface

Eclipse totality mask: SE can draw the full-solar-eclipse phase mask overlay (ShowEclipseMask); port has none

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg ShowEclipseMask; eng-gui "Eclipse totality mask"
- Port: none
- Check: SE has main-def.cfg:702 ShowEclipseMask (default false), the overlay-filter string "Eclipse totality mask" in strings_utf16:2712 among the grid/vector overlays, and control action ToggleEclipseMask (strings_utf16:4190); the port only renders eclipse shadows (README "Eclipses and shadows") and has no mask, totality or eclipse overlay in overlay.rs/hud.rs/settings.rs.

### 08-043 · Interface

Bottom info HUD option: SE can draw an old-style info HUD at the bottom of the screen when the bottom toolbars are hidden (BottomInfoHUD, off by default); the port always shows its own top info bar (date, rate, velocity, nearest, fps) and has neither SE's bottom toolbars nor this option

- Status: missing, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: main-def.cfg Interface BottomInfoHUD
- Port: none
- Check: main-def.cfg:547 BottomInfoHUD (default false, "draw old style bottom HUD when bottom toolbars are hidden") is confirmed, but the port is not without an info HUD: hud.rs:155-205 always draws its own top bar with date, time rate, velocity, nearest object and fps, and it has no bottom toolbars for the option to depend on.

### 08-042 · Interface

Debug toolbox: SE has a debug window with debug screens (Performance, Memory, Loader, Camera), histogram, bloom/depth/impostor/env-map views, octree/quadtree/bounding-box overlays, wireframe and level limits; port has none (env vars only)

- Status: missing, effort **L**, auditor confidence high
- SE evidence: main-def.cfg DebugOverlay 754-793; rtti seDebugToolbox; strings_utf16 "Debug toolbox" "Show Star octree"
- Port: none
- Check: strings_utf16:1155-1245 confirm the "Debug toolbox" window (rtti seDebugToolbox) with Performance/Memory/Loader/Camera screens, FPS meter, histogram, Wireframe, bloom/depth/impostor/envmap views, Land quadtree and Star/Galaxy/Cluster octree views, BBoxes and LandLimLevel/StarLimLevel/GalLimLevel, matching main-def.cfg DebugOverlay 754-791; the port has only a plain fps number in the top bar (hud.rs:183) and SE_* env toggles, with no debug window, overlay or wireframe.

### 09-018 · Graphics and settings

Loader and frame-timing options: SE lets the user pick LoaderMode (immediate / interleaved / asynchronous), MaxThreads, MultithreadedStartup, PreloadShaders / PreloadShipTextures, TimingStrategy 1 (time budget MaxTimePerFrame 10 ms, MaxTimePerFrameClean) and a "Frame timing mode" (real time vs fixed frame time, Export RealTime/RealTimeFPS for recording); port is fixed at SE's defaults (asynchronous task-pool loading, MaxTilesPerFrame tile budget, real-time stepping) with none of the alternatives selectable

- Status: missing, effort **S**, auditor confidence medium, amended by the checker
- SE evidence: main-def.cfg Loader 475-501, Export RealTime RealTimeFPS; eng-gui "Loader mode" "Immediate loading mode" "Interleaved loading mode" "Asynchronous loading mode" "Frame timing mode" "Real time mode" "Frame time mode"
- Port: crates/se_engine/src/graphics.rs:164
- Check: Port already behaves as SE's defaults (LoaderMode 2 asynchronous via AsyncComputeTaskPool in every builder, TimingStrategy 0 tile-count budget from MaxTilesPerFrame in graphics.rs:164, RealTime true), so only the selectable alternatives are missing; the UI strings are exe strings (strings_utf16.txt:2997-3003), not eng-gui.cfg, and RealTime/RealTimeFPS belong to Export (frame-time mode matters for video recording)

### 09-019 · Graphics and settings

Log file: SE writes system/se.log with optional time and thread stamps (LogTimeStamp, LogThreadStamp) and filters catalog and CSV loading messages (CatalogLogLevel 0-4 combined with each catalog's LogLevel, CsvLogLevel, CsvPatchWarning). Port prints Bevy log lines to the console only, with no log file and no catalog log levels

- Status: missing, effort **S**, auditor confidence high
- SE evidence: main-def.cfg Log 501-525; SpaceEngine/system/se.log
- Port: none
- Check: SE exe strings "se.log" (strings_ascii.txt:26401), LogTimeStamp, CatalogLogLevel and main-def.cfg Log 501-525 (LogTimeStamp, LogThreadStamp, CatalogLogLevel 0-4, CsvLogLevel, CsvPatchWarning); port main.rs uses DefaultPlugins' default LogPlugin (console only), no log file writer anywhere (only the opt-in SE_FRAMETIME_LOG frame-time file in system.rs:104), and catalog LogLevel keys are not used

### 09-017 · Graphics and settings

Stereoscopic 3D (non-VR): SE renders left/right eyes for cross-eye/parallel, horizontal or vertical stereopairs (optionally anamorphic interlaced, 3D_interlace.glsl), active shutter glasses, or anaglyph with chosen lens hues (3D_anaglyph.glsl, AnaglyphLeftHue/RightHue), with swap eyes, interpupillary distance, convergence, a stereobase that is saved with locations, Inc/Dec/Next/Prev/Reset stereobase keys, and links between stereobase and camera speed or time scale. Port has none (a place's Stereobase is only carried through, default 1.0, never used)

- Status: missing, effort **M**, auditor confidence high
- SE evidence: 3D_anaglyph.glsl:20-27; 3D_interlace.glsl; main-def.cfg Projection StereoscopicMode StereoInterlaced StereoSwapEyes StereoIPD StereoConvergence Anaglyph*Hue LinkStereobaseTo*; eng-gui "Stereoscopic 3D" "Cross eye" "Anaglyph" "Active (shutter)" "Interlaced" "Swap left/right" "Stereobase" "Convergence" "Left lens color"; def_generic_controller.cfg ToggleStereo3D CamIncStereobase
- Port: crates/se_core/src/places.rs:125
- Check: main-def.cfg Projection 354-383 (StereoscopicMode 0-6, StereoInterlaced, StereoSwapEyes, StereoIPD, StereoConvergence, AnaglyphLeft/RightHue, LinkStereobaseTo*), shaders re/se_shaders/3D_anaglyph.glsl and 3D_interlace.glsl, exe UI strings "Stereoscopic 3D"/"Cross eye"/"Anaglyph"/"Active (shutter)"/"Swap left/right"/"Left lens color" (strings_utf16.txt:3234-3268), def_generic_controller.cfg ToggleStereo3D/Cam*Stereobase; port grep for stereo/anaglyph/interlace finds only places.rs/se_script.rs carrying Stereobase through unused

### 10-033 · Scripts, audio, spacecraft

Thermal hull glow: materials with TempMap/TempMapMaxTemp (4) and the design's RadTemp (25 designs, radiators up to 1650 K) glow with blackbody colour from a Planck texture (THERM, ThermalParams). Port parses RadTemp and does not draw the glow

- Status: missing, effort **S**, auditor confidence high
- SE evidence: re/se_shaders/ship.glsl:61-63,392-401; ShipsTex.pak *.sml TempMap, TempMapMaxTemp; *.sss RadTemp
- Port: crates/se_core/src/ship.rs:186 (parsed only)
- Check: ship.glsl:61-63,402-412 (THERM: TempMap * ThermalParams.x sampled through the PlanckFunction texture and added to the hull colour), TempMapMaxTemp is read by FUN_1403e1d40/FUN_1404eb190, 3 ShipsTex.pak .sml files carry TempMap (AmS, fire_dragonfly, SHW), and 25 ShipsModels.pak designs carry RadTemp (max 1650 K); the port's ship.wesl has only diffuse + emissive map, and rad_temp is parsed (se_core/src/ship.rs:186) but never used in se_engine

### 10-035 · Scripts, audio, spacecraft

Physics stepping and gravity source: SE integrates ships on a fixed PhysicsTick of 0.015 s and pulls the ship toward its reference body (RefBodyID); the port uses variable substeps of up to 0.05 s cut from the frame time (ship time = frame dt times the sim rate, clamped to 10x) and sums the gravity of every body in the system

- Status: differs, effort **S**, auditor confidence low, amended by the checker
- SE evidence: main-def.cfg Physics PhysicsTick; docs/formats/spacecraft.md "Gravity acts from the reference body"
- Port: crates/se_engine/src/ship.rs:746-750,721-735
- Check: main-def.cfg Physics PhysicsTick 0.015 ("physics iteration time") and docs/formats/spacecraft.md:73-74 (gravity from RefBodyID) back the SE side, although no decompile shows the tick loop; the port does not take one whole-frame step: it splits ship_dt (frame dt * rate clamped 0.01-10) into ceil(dt/0.05) equal substeps of up to 0.05 s (ship.rs:886-889) and sums gravity_at over every SystemBody (ship.rs:720-733, documented in spacecraft.md:127-129)

### 10-030 · Scripts, audio, spacecraft

Script spline paths: SplinePath, PlaySplinePath, DeleteSplinePath and ClearSplinePaths (camera flight along spline paths, seSplinePathList, also used by the camera-path editor). Port skips them

- Status: missing, effort **M**, auditor confidence high
- SE evidence: exe command table splinepath, playsplinepath, deletesplinepath, clearsplinepaths; rtti seSplinePath, seSplinePathList
- Port: crates/se_engine/src/scripting.rs:577-580
- Check: Exe command table has splinepath, deletesplinepath, clearsplinepaths, playsplinepath and the "SE Spline Path (*.ssp)" filter, and Scripts.pak ships menu/Scene1.ssp (a SplinePath block with Catmull-Rom/B-spline data) though menu/Scene1.se does not use it; the port has no spline op or .ssp reader (no match for spline/ssp in se_engine), so these go to Op::Other and are skipped.

### 10-031 · Scripts, audio, spacecraft

Script capture and object commands: Screenshot, SavePNG, SaveTGA, SaveJPG, SaveDDS, SaveTIF and Watermark (with WaitTrigger "ScreenshotComplete"), plus ShowObject, HideObject, Show, Hide, Scene, Map and Clear. Port skips them; its ScreenshotComplete fires at once

- Status: missing, effort **M**, auditor confidence high
- SE evidence: exe command table screenshot, savepng .. savetif, watermark, showobject, hideobject, show, hide, scene, map, clear
- Port: crates/se_engine/src/scripting.rs:577-580,1053
- Check: The exe script command table (name + handler pointer pairs, read from SpaceEngine.exe .rdata) contains screenshot, savepng/tga/jpg/dds/tif, watermark, showobject, hideobject, show, hide, scene, map and clear; the port's se_script.rs Op enum has none of them, so they fall to Op::Other and are skipped with a warning (scripting.rs:577-580), and "screenshotcomplete" returns Check::Yes at once (scripting.rs:1053); no shipped Scripts.pak script uses them, so visibility 1 stands

### 10-032 · Scripts, audio, spacecraft

Script locale and utility commands: SetLocale, PrintTranslations, ExportLocale and ImportLocale (CheckLoc.se), PrintVars, PrintNames, PrintFont, ExportFont, PrintTexCacheLog, DBSort, OptimizeSM, ReloadConstellations, DisplayMode, ToggleStereoscopic3D, DebugGen and Rebuild; the port skips them as unsupported (Edit left out: editor, out of scope)

- Status: missing, effort **M**, auditor confidence high, amended by the checker
- SE evidence: Scripts.pak CheckLoc.se; exe command table setlocale .. edit
- Port: crates/se_engine/src/scripting.rs:577-580
- Check: The exe command table has setlocale, printtranslations, exportlocale, importlocale, printvars, printnames, printfont, exportfont, printtexcachelog, dbsort, optimizesm, reloadconstellations, displaymode, togglestereoscopic3d, debuggen, rebuild and edit, and CheckLoc.se uses SetLocale/PrintTranslations; the port parses none of them and skips them as Op::Other (it has a locale module, se_core/src/locale.rs and i18n.rs, but no script hook into it); "Edit" opens the editors, which are out of scope, so it comes off the list

### 10-034 · Scripts, audio, spacecraft

Ship persistence: SE saves ships between sessions in spacecraft.cfg (ShipID, SystemID, RefBodyID, TargetShipID, DockedShipID/DockedPort, ship state such as WarpBoost) and restores them, also through the game state (seGameState::ApplyToCamera spacecraft id). Port forgets the ship on exit

- Status: missing, effort **M**, auditor confidence medium
- SE evidence: exe ConfigFileSpacecraft, "spacecraft.cfg", seSpacecraftsList::SaveSpacecraftsConfig, "ShipID %i"
- Port: none
- Check: The exe has "spacecraft.cfg" (FUN_1400402e0), ConfigFileSpacecraft, seSpacecraftList::ReadShipsConfig ("has no ShipID"), seSpacecraftsList::SaveSpacecraftsConfig, RefBodyID/TargetShipID (FUN_140531d60) and "seGameState::ApplyToCamera(): Unknown spacecraft id %i"; the port has no ship save or load (no spacecraft.cfg, ShipID or ship save code in crates/; ship.rs only spawns and despawns through ShipCommand)


## Fixed since the audit

- **01-001** (Stars, fixed 2026-10-29): star point profile and magnitude->brightness/size model: SE draws a Gaussian core plus a 1/(PointFlareDecay*r^... Fix: `stars.wesl` uses SE's model: brightness StarBrightMin x PointOverbright x 10^(0.4 (StarMagnLimit - m)), above 100 compressed to 100^0.75 b^0.25, faded out at 0.03; a Gaussian of 2 sigma^2 = PointScale^2 x h/1080 px^2 out to ln(500 b). Not done: the flare wing (SE passes CosmHorizon as its decay, which leaves it near zero).
- **01-002** (Stars, fixed 2026-10-29): star colours: SE greys out faint stars (GrayLevel = pow(clamp(mag/GrayBright),3), PointDesaturation 0.1) with ... Fix: Dim stars grey out as (b / PointDesaturation)^3 and StarColorSaturation (1) sets the colour; settings keys are SE's (`StarMagnLimit`, `PointOverbright`, `PointScale`, `StarColorSaturation`, `PointDesaturation`). StarColorShift (2026-10-29): colours at the temperature x 10^-shift (FUN_1401a60d0: powf(10, -shift)), in `stars.wesl` from the baked colour through a blackbody table; Settings slider, saved key. Not done: its Ctrl+Shift+Home/End keys (the port has no star-setting keys yet).
- **01-003** (Stars, fixed 2026-10-29): star surface close up: SE builds each star's surface from per-star parameters (tg_sun_height/tg_sun_glow): gra... Fix: Star surfaces (star_surface.wesl, surface_gen.rs `star_surface`): SE's surface generator star branch (FUN_1402f37c0, object type 3) gives each star its granulation scale from surface gravity (giants and carbon stars 4 g / sqrt(Teff) with flows and 15 octaves; dwarfs sqrt(Teff/6000) f^2 / sqrt(g/g_sun)), distortion, granule bottom/top temperatures (0.9/1.1 giants, 0.95/1.05 dwarfs) and spots on dwarfs (density and size by spectral type, latitude band above 2700 K, filaments); the shader evaluates SE's GlowMapSun with SolarSpotsTempNoise per pixel (SE's hash Perlin and its 64^3 noise texture), applies SE's limb darkening as 0.86x..1.1x temperature and colours by a blackbody. Not done: the height relief (HeightMapSun), gravity darkening of oblate stars and the brown-dwarf cloud mix; the absolute brightness keeps the port's calibration (SE's Planck table scale is not decoded) and the limb darkening amount (SurfParams1.y) is assumed 1.
- **01-004** (Stars, fixed 2026-10-28): visiting Vega or Altair: after "goto Vega/Altair" the port drops to seconds-to-minutes per frame (no screensho... Fix: `Catalog::system_members` walks objects and names once each (a child named like its parent, planets/Stars.sc "Vega" under "Vega/ALF Lyr", listed itself 100 000 times); Vega and Altair render at full frame rate.
- **01-005** (Stars, fixed 2026-10-28): stars within 0.02 pc of the camera that are not members of the active system are invisible: the starfield hide... Fix: The star fields hide only the active system's own point, by position (`StarParams::hide`), not every star within 0.02 pc; HIP 76566 shows next to HIP 76563.
- **01-006** (Stars, fixed 2026-10-30): planets lit by several stars: SE lights a body with up to 4 suns (colour, position and shadow of each); port l... Fix: SE gathers up to MAX_LIGHTS 4 stars per body, brightest flux first, ends the list at the first one past PlanetLightingFadeEndMagn, fades those between the begin/end magnitudes and normalises each to (F / max(F_begin, F_max))^0.35 (FUN_1404bf5c0, the port's LightingFade::planet_intensities); BodyLighting keeps the brightest as before plus up to 3 ExtraLight (direction, illumination colour times intensity, ring colour cut at RingsLightingFadeEndMagn). Planet surfaces, clouds, skies, rings and catalog 3D models are lit by each (planet.glsl/sky.glsl/rings NLights loops): direct light with the Hapke term, sky irradiance, day ambient, haze, ring and eclipse shadows on each star's own path. Checked: Earth unchanged; Kepler-47 c gains its red dwarf's light (disc R +8%, B +3%); no frame-time cost. Not ported: eclipses of the further stars use the brightest star's angular size, their sun discs in the sky shader, water glints, thermal heating and city-light switching by them; ship hulls (10-025) still use one star.
- **01-007** (Stars, fixed 2026-10-29): star lighting of planets: SE fades a light source's effect on planets between apparent magnitude -7 and -5 (ri... Fix: Starlight on planets as SE sets it up (FUN_1404bb110 thresholds, FUN_1404bf5c0 lights): each star's flux at the body (L_sun/AU^2) fades linearly between PlanetLightingFadeBeginMagn -7 and EndMagn -5 (the flux of that apparent magnitude, SE's FUN_1401df4c0 at 1 AU) and is then normalised as (F / max(F_begin, F_brightest))^0.35, so a planet under a star brighter than -7 is lit at 1 wherever it is (Neptune 1.0, was 0.36) and a body with no star is unlit (was lit at 1.0); rings drop a star fainter than RingsLightingFadeEndMagn -10 (SE never reads RingsLightingFadeBeginMagn); values read from main-def.cfg (system.rs LightingFade). Light colour per FUN_1404d6970: the star's colour (brightest channel 1) pulled towards its luma sqrt(.299r^2+.587g^2+.114b^2) by SunIllumSaturation (0.5, range 0-2) or, with IllumSaturationAuto (default on), SE's temperature curve FUN_1401a5270 (colour mode 2: 0.87 at 2000 K, 0 near 6000 K, 0.45 at 10000 K, 0.75 above 18700 K); both settings persisted under SE's keys and shown in Settings. Planet sprite magnitudes now use the physical flux. Not ported: SE's brightness factor for stars under 2000 K, colour mode 3's curve, and lighting by several stars (01-006).
- **01-015** (Stars, fixed 2026-10-29): procedural star blocks pop in: SE fades newly generated blocks of stars in over StarFadeDuration 0.5 s; port s... Fix: New blocks of procedural stars fade in as SE's do (FUN_14048dd70: a block's brightness factor rises by dt / StarFadeDuration a frame from 0 once it is ready, capped at 1; StarFadeDuration 0.5 s read from main-def.cfg, at least 1 ms): a finished octree node is drawn with its own copy of the shared star material whose colour multiplier (tint) runs 0 -> 1, then switches to the shared material (procstars.rs stream_nodes, stars::block_fade_seconds).
- **01-016** (Stars, fixed 2026-10-30): star magnitude limit and zoom: in SE, zooming works like a telescope and raises the stars' limiting magnitude ... Fix: Zoom works like a telescope as in SE: its camera keeps a zoom factor (FOV / DefaultFOV)^-1.5 (FUN_140572300, seCamera +0x1390; DefaultFOV 45 read into +0x1360 by FUN_140280ec0) that multiplies the brightness coefficients of stars, planets and clusters every frame (FUN_1404c51a0, mono view only); StarSettings.zoom follows the camera and star_params raises the shader's limit by 2.5 log10(zoom) (catalog, procedural, cluster and planet points). Loading follows the zoomed limit: SE keeps a star node while its brightest star could show under the coefficient capped at magnitude 20 (FUN_1404a0e80, FUN_1404c4610) and, under StarFrustumClipFOV 30 (main-def.cfg), only inside the frustum (FUN_14047b670), so StarOctree::wanted_nodes reaches 10^(0.2 dm) further inside the view cone (at most 512 extra nodes a level; all around at most 2x without a cone) and cluster nodes use the zoomed limit inside the cone. The camera toolbar shows SE's 'Mag limit' = StarMagnLimit + 1.0587 ln(zoom) (FUN_1404c27c0; typing sets the limit back, FUN_14024d6e0); SE's keys ] / [ (IncAll/DecAllMagnCoef, +-0.1), Ctrl+] / [ (stars), P (ResetAllMagnCoef) from its key table FUN_1405ba8f0 with a splash message; the ship's warp boost moved from [ ] to SE's Alt+= / Alt+-. Zoomed in, procedural galaxies, clusters and nebulae are promoted only inside the view (all around they grew past the size thresholds and cost 80 ms frames). Checked against the tutorial's 'Mag limit' numbers (internal 7.7-9.4 for 10 at FOV 10-30; port 7.6-9.36); at 0.2 deg the frame median is 7 ms, worst 18 ms. Not ported: galaxy and nebula points brightening with zoom, separate planet/galaxy limits (Shift and Ctrl+Shift keys).
- **02-001** (Star clusters, fixed 2026-10-28): Catalog cluster far sprite: SE draws every catalog (and extragalactic catalog) cluster as a star-like particle... Fix: Catalog clusters are SE's star-like particle (`procsystems.rs` setup, `galaxy_far.wesl` ptype 4), coloured by `Color` or SE's default by type, open clusters at 0.25; `Color (0 0 0)` draws none. Omega Cen shows from Earth.
- **02-002** (Star clusters, fixed 2026-10-28): Far-to-near switch rule: SE renders member stars once the cluster's angular radius exceeds 8 px (x max(1, heig... Fix: Member stars from 8 px x max(1, h/1080) of radius (`clusters::members_range_pc`, also for procedural promotion); every cluster particle fades out from 1 to 100 px x h/1080. Not done: SE's brightness cut on members (d^2 ClusterBrightMin < K L).
- **02-003** (Star clusters, fixed 2026-10-29): Member-star LOD octree: SE gives each cluster a 5-level star octree (nodes "SC cluster-level-node", root half-... Fix: Cluster member octree (clusters.rs): SE's structure (FUN_140543c70 / FUN_140540220 / FUN_1401d7300): five levels below a root cube of half-size the cluster radius, level d holding one of SE's five equal-probability bands of Mv ~ N(3, 6) from -8 to 30 (-8, -1.5, 1.8, 4.7, 8.2, 25); a node loads while the camera is inside its sphere or its band's brightest star seen from the sphere's edge is above the star magnitude limit (StarMagnLimit + PointOverbright), at most 16384 stars per node, brighter nodes first, so faint members appear as you approach and big globulars reach millions of stars around the camera; counts scale with StarDensityInCluster. Not done: SE's own per-node star counts and member generator (CreateClusterBlock, FUN_1401d6c60) are not decoded, so band populations come from the port's Salpeter/turn-off model and Plummer profile.
- **02-004** (Star clusters, fixed 2026-10-28): Catalog defaults for missing keys: SE draws age N(11,3) Gyr for globulars (152 of 154 in Globular.sc have no A... Fix: Missing age, radius and magnitude come from SE's ranges (`catalog::complete_cluster`: globular age N(11,3), open N(0.15,0.1) ...), seeded by the cluster's name instead of SE's random stream.
- **02-005** (Star clusters, fixed 2026-10-28): Luminosity of open clusters without magnitude (1113 of 1298 catalog clusters): SE uses L = 10*R^3*StarDensityI... Fix: Open clusters without a magnitude use SE's L = clamp(10 R^3, 1000, 2e5) capped at ClusterCatMaxAppMagn 8.1 seen from the Sun (Pozzo 1: 44.5 L_sun).
- **03-001** (Galaxies and nebulae, fixed 2026-10-29): Galaxy front-texture plane: SE draws each sprite-model galaxy's full-resolution FrontTexture (`<name>.png`, 20... Fix: Sprite-model galaxies draw their FrontTexture (with mipmaps) as a disc over the sprites (`galaxy_plane.wesl`, after SE's galaxy_plane.glsl: faded within a model unit of the eye and edge-on, colour ^ 1/DSOGamma, premultiplied so its dust lanes hide the sprites behind); M 51's arms and pink knots, the Milky Way's bar and arms from outside. The disc takes 70% of a face-on model's calibrated brightness and the sprites the rest (port calibration; SE draws both at the model's brightness in units the port does not reproduce).
- **03-014** (Galaxies and nebulae, fixed 2026-10-29): Fade-in of new procedural galaxy blocks: SE fades newly generated galaxy blocks in over StarFadeDuration (0.5 ... Fix: New blocks of procedural galaxies fade in the same way over StarFadeDuration (SE's 'blocks of stars and galaxies'): each new node gets its own copy of the far-sprite material, kept in step with the shared one by sync_material, with FarParams.cam_pos.w = 1 - fade scaling the brightness gain in galaxy_far.wesl (0 for all other far sprites), then rejoins the shared material (procgalaxies.rs).
- **04-001** (Planet surfaces, fixed 2026-10-29): Hapke shading and opposition surge on airless bodies: SE lights the ground with HapkeBRDF (catalog Surface key... Fix: SE's HapkeBRDF (terrain_pbr.glh) on the direct light, mixed in by Surface Hapke (or Lommel), with SpotBright/SpotWidth and the CB spot; the full Moon is evenly lit.
- **04-002** (Planet surfaces, fixed 2026-10-29): Oblate and triaxial bodies: SE draws bodies as ellipsoids from the catalog Oblateness (Saturn 0.098, Jupiter 0... Fix: `BodyRecord::shape` (Dimensions, Oblateness scalar or per axis): terrain, cloud and aurora meshes, the body sphere and the sky shell are drawn as the ellipsoid (the shell turns with the body and works in its own frame); ground clearance, terrain LOD, shore and surface placement work through the shape. Not done: asteroid models and the eclipse/ring shadows stay spherical.
- **04-003** (Planet surfaces, fixed 2026-10-29): Surface tint keys ModulateColor and ModulateBright: SE multiplies the surface colour by the catalog Surface Mo... Fix: The colour map takes Surface ModulateColor x ModulateBright (`PlanetUniform.modulate`).
- **04-004** (Planet surfaces, fixed 2026-10-29): Procedural terra colour maps: SE paints procedural rocky worlds with tg_terra_color (biome by height band via ... Fix: Procedural surface colours follow SE's own colour shaders (planet.wesl `se_terra_mix` / `se_selena_mix`, from the decrypted tg_terra_color.glsl / tg_selena_color.glsl): terra worlds get SE's climate (Fbm-distorted latitude through the tropic band, or the substellar distance on tidally locked worlds; height temperature gradient, snow line, ice caps; the flatland distortion by ridged multifractals of hillsFreq/hillsMagn and colorDistFreq), the base surface by SE's height bands (bottom, shelf, beach at sea level, lowland, upland, rock), steppe/forest/grass by their climate ranges where humid enough (PlantsBlendCoeff with the transition fractal), snow above climate 0.9, layered cliff rock on slopes over 0.35, and SE's albedoVary with ColorVary; airless worlds (SE's selena generator) get the height bands, the slope- and young-surface-modulated albedo variation, drivenDarkening and whitened ice caps. SE's noise (hash Perlin, its Fbm/Fbm3D/RidgedMultifractal) with octaves under a pixel dropped. Not done: cracks, volcano masks, rayed craters, the hills' Jordan turbulence and SE's height-aware BlendMaterials (the port blends its two strongest slots).
- **04-005** (Planet surfaces, fixed 2026-10-29): Thermal emission and lava glow: SE gives hot bodies a temperature map (tg_terra_glow, tg_selena_glow, tg_aster... Fix: Thermal emission (planet.wesl `thermal_emission`, SE's planet.glsl THERM): the temperature map as SE's tg_*_glow generators make it (Neptune/Jupiter/Chthonia-bulk bodies glow all over at surfTemperature x (1 - 0.2 height); rocky worlds where lava covers them, with SE's glob/vary noise and lava-coverage terms), the sun's heating T = (0.010673289 L/d^2 n.l + T^4)^(1/4), ThermalEmissionShift; the colour is a blackbody's radiance relative to the star's surface at 610/550/465 nm, scaled by the star's surface radiance implied by the port's irradiance. WASP-12 b and Kepler-10 b now glow orange on their night sides. Assumed: surfTemperature = the body's average temperature; SE's Planck texture and thermal brightness (SurfParams4.w) are not decoded. Not done: volcano calderas (VolcanoGlowNoise), the glowing atmosphere (GlowColorAtmo), clouds hiding the glow.
- **04-006** (Planet surfaces, fixed 2026-10-29): Procedural gas and ice giants: SE builds bands from HeightMapCloudsGasGiant (noise zones by stripeZones, equat... Fix: Procedural gas and ice giants (planet.wesl `gas_giant_albedo`): SE's HeightMapCloudsGasGiant per pixel (Noise zones by stripeZones, equatorial oval and polar round cyclones from cyclone*/cycloneMagn2/cycloneLatitude2, TurbulenceGasGiant vortices twisted by stripeTwist, Fbm stripes by cloudsFreq/cloudsOctaves and stripeFluct; SE's hash Perlin noise and spherical Fibonacci cells), coloured as tg_gasgiant_color from the giant preset's 8-colour CloudsPalette (height x colour, then gamma); giants now get a biome preset (SamplesPresets jupiter_*/neptune_*). The stripe and cyclone parameters follow FUN_1402f37c0's giant branch distributions (surface_gen::giant_clouds). Assumed: two undecoded inputs (polar cyclone latitude factor, a cold Jupiter's equatorial cyclone octaves), the palette lookup's row mapping, and the branch for bodies flagged at +0x38; the draws do not follow SE's order. Not done: the giants' fog layer (HeightMapFogGasGiant).
- **04-007** (Planet surfaces, fixed 2026-10-30): Planet shine (secondary lighting): SE lights night sides with light reflected from nearby bodies (earthshine o... Fix: Planet shine as SE does it: bodies drawn as discs are lit by the planets and moons around them as further lights among their up to 4 (FUN_1404bc1a0 gathers them, FUN_1404bbf40 picks the sources: the body's moons, up its parents a planet itself, and at the first star or barycentre its other children). Each reflects, per star lighting it, F (R/d)^2 albedo (0.36 without one) Lambert(a) clamp(pi - asin(R/d) - a, 0, 1) times its opposition surge (Hapke, SpotBright, SpotWidth), the total scaled by the luminance of its colour (PlanetShineMode 1) and by PlanetShineBright; its colour is its own times the stars', brightest channel 1 (FUN_1404da600). They are faded like stars but never set the normalisation (only stars do, FUN_1404bf5c0), so earthshine lights the Moon's night side at about 4% and Jupiter's moons its night side. EnablePlanetShine / PlanetShineBright are read from main-def.cfg. Bodies inside their star (the port leaves 'Oumuamua at the Sun) are skipped; the cost is about 0.2 ms with 2566 Solar System bodies. Not ported: SE's preview renders for the colour (the sprite colour is used), eclipses of the shine, the rings' host-planet light (05-012), the TogglePlanetShine key and the settings entries.
- **04-008** (Planet surfaces, fixed 2026-10-30): Lighting by several stars and its magnitude fade: SE lights a surface with up to 4 stars (MAX_LIGHTS 4 loop) a... Fix: Surfaces lit by up to 4 stars with SE's magnitude fade (see 01-006): SE gathers up to MAX_LIGHTS 4 stars per body, brightest flux first, ends the list at the first one past PlanetLightingFadeEndMagn, fades those between the begin/end magnitudes and normalises each to (F / max(F_begin, F_max))^0.35 (FUN_1404bf5c0, the port's LightingFade::planet_intensities); BodyLighting keeps the brightest as before plus up to 3 ExtraLight (direction, illumination colour times intensity, ring colour cut at RingsLightingFadeEndMagn).
- **04-011** (Planet surfaces, fixed 2026-10-30): Terminator on airless bodies: SE uses a sharp per-pixel horizon shadow that depends on height (HorShadow = cla... Fix: SE's sharp, height-aware horizon shadow on every lit body: HorShadow = clamp((cosHor + N.L) * 500, 0, 1) with cosHor = sqrt(1 - rr^2), rr = clamp(1 / (1 + height / R), 0, 0.9999) (planet.glsl:1087-1095), for the brightest star and the further lights alike, so the terminator is sharp and raised points stay lit past it (was a smoothstep(-0.08, 0.12) of the sphere normal). Checked on the Moon at first quarter.
- **04-012** (Planet surfaces, fixed 2026-10-30): Ambient light: SE adds DayAmbient (catalog Surface key) only on the lit side and a user "Ambient lighting" lev... Fix: SE's AmbientLighting (default 0, 0..1 in steps of 0.05, rounded to them as FUN_1404c12c0 does) replaces the port's fixed 1.5% albedo ambient: it adds albedo/pi x level in the sun's units, day and night, and Shift+. / Shift+, (> / <, IncAmbient / DecAmbient from SE's key table FUN_1405ba8f0) change it with SE's two-decimal message. DayAmbient on the lit side was already ported. Night sides are now dark as in SE apart from planet shine and city lights. Not ported: the Settings window entry and saving the level.
- **05-001** (Atmospheres, rings, comets, fixed 2026-10-29): Clouds are not lit through the atmosphere: SE draws each cloud layer through the planet shader (ACLOUDS), so t... Fix: `clouds.wesl` lights clouds as SE's ACLOUDS: sun light reddened by the transmittance and cut at the planet's horizon at cloud height, sky irradiance x SkyLight, DayAmbient (default 0.07, SE's loader), Hapke mix, SubsurfScatBright/Power (default 0), aerial perspective over the cloud, SE's 5% opacity cut; the night side is unlit. Relief: a layer with a BumpMap (Earth's is its cloud map) tilts the cloud-top normal by the density's screen-space gradient times BumpHeight.
- **05-002** (Atmospheres, rings, comets, fixed 2026-10-29): Cloud layers and Clouds catalog keys: SE draws every Clouds block as its own layer (Titan 3 orange haze layers... Fix: Every Clouds block is its own layer (`terrain::CloudLayer`; Titan's three haze layers at 20/30/50 km, Mars's two textured and one procedural), NoClouds honoured. Per layer: height, map, Opacity, ModulateColor x ModulateBright, DayAmbient, Hapke, SubsurfScat, BumpMap/BumpHeight, DiffMapAlpha "None" (the map is an opaque colour layer: Venus), FlipMap and Velocity (the layer's shell turns about the pole with the simulation time, signed by the body's rotation, each turned layer with its own patch selection); SE's body-wide Coverage/mainFreq/mainOctaves (FUN_14046dde0 writes them to the body: the last block's win) for the procedural ones, Coverage 1 closing the layer; SE's 1 km close-camera fade; textured-surface bodies get their procedural layers (Titan's orange haze); nodes are culled against the highest layer. Not done: GlowMode/GlowMap, TempMap, RingsWinter; ground shadows come from the first textured layer that does not turn.
- **05-003** (Atmospheres, rings, comets, fixed 2026-10-29): Procedural cloud generator: SE's tg_clouds_height/color/glow use HeightMapCloudsTerra with cyclones (cycloneMa... Fix: Procedural clouds follow SE's terra cloud generator (clouds.wesl `se_terra_clouds`, from tg_common.glh HeightMapCloudsTerra / CycloneNoiseTerra / TurbulenceTerra / FbmClouds and tg_clouds_color.glsl): cyclones in random spherical Fibonacci cells per octave (cycloneMagn/Freq/Density/Octaves), two octaves of small vortices, the Coriolis twist by latitude zones (stripeZones, stripeTwist), the flow distortion of FbmClouds (x pi, x 0.333 per octave, cloudsFreq, cloudsOctaves) against the coverage, SE's `saturate(h^2 3.5 + h)` opacity; on tidally locked worlds one giant cyclone at the substellar point and a clear night side. The parameters are SE's generated ones (surface_gen), and bodies SE's completion gives clouds (above 0.05 atm) without a Clouds block get SE's default layer. Not done: the cloud palette colours (GetCloudsColor) and the cloud glow map; a lattice artifact can show over flat lowland seen from afar (Kepler-62 e).
- **05-004** (Atmospheres, rings, comets, fixed 2026-10-29): Atmosphere thickness: SE gives each body its own visual atmosphere height (FUN_140448ef0: with AtmoHeightHarb,... Fix: Each atmosphere is drawn at its body's visual height (`AtmosphereGpu::with_height`, SE's `Radiuses`: the model's tables read at the altitude's fraction of the height, its scale heights stretched and coefficients shrunk so the optical depths stay the model's): the catalog Height (FUN_140448ef0 keeps it), else SE's AtmoHeightHarb formula p^0.1 x 7.8 scale heights, at least 2 x SE's default BumpHeight, at most 0.1 R (`climate::se_visual_atm_height_km`; generated rocky worlds now use it too, giants 15.6 scale heights). Titan 200 km (was 60), Jupiter 300 (was about 660), Pluto 300. The haze over a cloud blends SE's horizon fix across the gap between the ground horizon and the cloud's own (it showed a lump on Titan's limb). Assumed: SE keeps the optical depth when it stretches a model.
- **05-005** (Atmospheres, rings, comets, fixed 2026-10-29): Atmosphere look keys: in SE, Bright scales the scattering on both the sky and the ground (the disc haze and th... Fix: SE's calibration (main-def.cfg): scattered light x Bright x hdrAtmoScatterBright 1, surfaces x hdrPlanetSurfaceBright 5, ambient x hdrAtmoIllumBright 1 (`ATM_SCATTER_SCALE`, `ATM_SUN_SCALE`, `ATM_AMBIENT_SCALE`). `AtmParams.look`: Opacity mixes every transmittance towards 1, Hue/Saturation adjust the inscatter (SE's HSV AtmoColAdjust), SkyLight scales the sky irradiance on the ground and clouds (SE's ambient form, not divided by pi), plus the surface DayAmbient; generated atmospheres pass their opacity and SkyLight too.
- **05-006** (Atmospheres, rings, comets, fixed 2026-10-29): Seeing through the sky: SE adds sky inscatter on top of the scene and dims what lies behind by the per-channel... Fix: The sky shell draws in three passes (`atmosphere::SkyPass`): the empty background (far-plane depth test) fades by sky brightness, everything behind the sky is multiplied by the per-channel transmittance (`atmo_transm.glsl`), then the inscatter is added. Stars and the Milky Way redden at the horizon. With SE's calibration (main-def.cfg: scattered light x hdrAtmoScatterBright 1 x Bright, sunlit surfaces x hdrPlanetSurfaceBright 5; the port had x5 on the sky too) the daytime sky is a mid blue and the first-quarter Moon shows in it; the haze on ground and clouds now takes Bright as well (that part of 05-005).
- **05-007** (Atmospheres, rings, comets, fixed 2026-10-29): Ring lighting model: on the lit face SE uses a Hapke x Lambert phase with an opposition surge (Hapke, SpotBrig... Fix: SE's ring lighting (rings.glsl): the lit row premultiplied by its alpha scatters with Hapke x Lambert by phase angle and the French et al. opposition spots (SpotBright/SpotWidth, SpotBrightCB/SpotWidthCB), the back row forward-scatters with Mie g=-0.95, both turned into optical densities (Density, Opacity, SelfShadow; PlanetShadow) and marched in 8 steps through a slab whose density falls as exp(-3 x height) and SE's self-shadow towards the Sun, so the unlit face shows the dense B ring dark and the thin rings glowing; Brightness, FrontBright and BackBright linear (were square-rooted); RingDetailNoiseScale radial detail noise where resolved. Assumed: the slab's bounding sizes (RingsSize), which DensityParams channel takes which key, the detail noise's scale.
- **05-013** (Atmospheres, rings, comets, fixed 2026-10-30): Several light sources: SE lights skies, surfaces, clouds and rings by up to 4 stars (MAX_LIGHTS, NLights loop;... Fix: Skies, surfaces, clouds and rings lit by up to 4 stars with SE's magnitude fade, rings dropping a star past RingsLightingFadeEndMagn (see 01-006). Not ported: the rings' fifth light (HostLightIndex, the planet's own light, planetshine 05-012) and eclipses between the stars themselves.
- **05-014** (Atmospheres, rings, comets, fixed 2026-10-30): Comets: SE draws tails as animated particle sprites (catalog Particles count). Gas particles are pushed anti-s... Fix: SE: FUN_140484400 draws comet_tail.glsl particle sprites from FUN_1404d4100; FUN_1404858f0 builds the VBO (Particles x 4 corners: random direction, phase bytes, kind 0 gas / 1-255 dust from a 2-octave value-noise fBm); FUN_140485750 sets Length = MaxLength (1 AU/r)^SlopeParam; FUN_140486280 fills missing tail values for every comet (MaxLength from mass, Particles 1000-3000 from MaxLength, GasToDust 0.25, Gas/DustBright U-split of U(0.01,0.05), colours, AbsMagn tn(8,4,-2,20), SlopeParam tn(4,2,2,20)). Uniforms: ScaleTime (Length km, frac(dt/0.6 d), 1-(q/r)^0.25, 1), per-particle colour 5000*CometTailBright*hdrCometTailBright*Bright/N, CometVel km/s, EyePos/SunPos AU (SunPos.w = 1), SunCol. Ported: comets.rs (uniforms, lazy particle meshes, SE defaults via se_core::procgen::tail_defaults) and comet.wesl (comet_tail.glsl line by line, HG g=-0.3). The old coma sphere and two tubes are gone; CometTailBright/hdrCometTailBright read from main-def.cfg. Port fix needed: the sprite tangent normalises before cross() (render metres overflow f32 otherwise). Spawn fix: spawn_missing_comets now waits until the system loader is done (it ran before the loader and could spawn comet bodies twice), spawn_tails gives one tail per comet. Checked: Halley 1986-03-10 and Hale-Bopp 1997-04-01 from the side (blue gas tail straight anti-sunward, whiter dust tail curved behind), Halley's coma close up, Halley 2024 (35 AU, no tail), AltairC119 (procedural). Frame times vs baseline in .audit/work/comets/ft_*. Not ported: SE's fade-in by the comet's apparent brightness (FUN_1404da600 output x DAT_14080b578, a runtime scale) and the impostor FBO path (EnableCometFBO/EnableCometImpostors); the port draws a tail once it is >= 2 px. Catalog comets draw SE's missing values on a stream seeded by name (SE uses its thread RNG at load). Length capped at 1e4 AU near the star (SE uncapped). NoCometTail is honoured (SE's FUN_140486280 would re-enable it).
- **06-001** (Procedural generation, fixed 2026-10-29): Catalog stars without catalog planets get no planetary system: SE runs CreateProcedural (planets, moons, belts... Fix: A catalog system without catalog bodies whose one real star has no NoPlanets true gets the port's generated system on arrival (Vega: 25 bodies), seeded by the star's name. Not done: systems of several stars without planets stay empty.
- **06-002** (Procedural generation, fixed 2026-10-29): Procedural planet architecture is designed, not SE's: SE gives each single star u%10+5 (5-14) planet slots (ci... Fix: Procedural planetary systems use SE's architecture (procgen/se_layout.rs, from FUN_1404f9210 / FUN_140500700 / FUN_14050a9f0 / FUN_140501f60 / FUN_140504e10 / FUN_1401aa500 / FUN_1404df8a0 / FUN_1401abd00 / FUN_1404fd2a0, probabilities from main-def.cfg SolarSystemGenerator matched to their exe globals): 5-14 slots out to a sphere of influence of tn(40,8,10,1000) AU per solar mass, the inner limit at 1.05 R*, T_eq 800 K (3200 K with hot planets: giants or 6.2%) or a mass term, geometric steps U(1.2,2.7) with e=/N(0,0.06)/ (7% U(1.7,2.2) with e=/N(0.2,0.1)/), SE's inclinations, planets above 3200 K dropped, gas giant 12% / ice giant 25% / terra with SE's log-normal masses capped by the disk (1.2% of the star), ProbColdTerra, SE's drawn giant thresholds and rocky composition (written to Interior.Composition, which the completion now keeps), SE's radius fits (giant fit in M_J and T_eq; Seager-style tables by ice/carbon/silicate share), dwarf planets (<= 2000 km) with SE's 100+ asteroid belts between the neighbours' Hill zones (1000 per system at most), SE's moon counts (N(60,20) giants, N(20,8) mini-Neptunes, N(10,8) solid planets, N(3,2) dwarf planets); SE's AU-vs-parsec overlap guard reproduced. Not ported: SE's moon generator (FUN_1404fdc90, the port places SE's count), asteroid sizes (FUN_140502710), comets, debris rings, the star's own C/O and metallicity draws (solar assumed). Giants and brown dwarfs no longer get a default cloud layer (SE's tg_gasgiant/tg_browndwarf height shaders leave cloud layers empty; the port's fallback clouds aliased into a lattice on them).
- **06-003** (Procedural generation, fixed 2026-10-29): Procedural bodies bypass SE's body-completion chain that the port already has for catalog bodies: SE runs bulk... Fix: Procedural planets and moons go through SE's completion chain (procgen `emit_body`): they are written as SE's system generator leaves them (class, mass, radius, orbit, rotation, life, rings, aurora), so like any catalog body without them SE's chain (climate::complete: bulk composition, albedo, tidal heating, volcanism, atmosphere/greenhouse/ocean solve, look, temperature model and class word) and SE's surface generator (surface_gen) give them their atmosphere, ocean, clouds, surface and class. Not done: their rotation is still the port's (SE's derived spin is not written back into the rotation model), life is still the port's decision, and asteroids keep the designed blocks.
- **06-004** (Procedural generation, fixed 2026-10-29): No multiple-star procedural systems: SE splits ~50% of systems into binaries, 11% of those into triples, 22% i... Fix: Procedural stars become SE's multiple systems (procgen/se_multiple.rs, from seSolarSystem::CreateProcedural FUN_1404f7c40 and the binary branch of FUN_1404f9210): A/B with ProbBinaryStar 0.5 (0.95 for luminosity code 10), A and B again with ProbTripleStar 0.11 and their halves with ProbQuadrupleStar 0.22 (up to 8 stars, SE's suffixes A..BBB), sub-splits capped at 0.3 of the component's sphere of influence; companion white dwarfs / neutron stars / black holes (ProbCompanionWD/NS/BH, SE's mass draws, a too-old primary reset to the mass whose lifetime is the age), brown-dwarf companions (ProbBrownDwarf, x0.2 after each), else the light split by SE's mass-luminosity law (FUN_1401dd880); separations log-normal up to the cap (tn(log10 cap/1.84, 2.5, -2, log10 cap) AU, at least 2.5 radii), e = tn(0.5,0.5,0,0.89) damped for close pairs, a random barycentre plane; planets per component (rand%10+5 slots inside sphere of influence periapsis/(sqrt(q/0.05)+1)) and circumbinary planets of the root barycentre (FUN_14050a890 inner edge 3 apoapses). The engine adds such a star as a barycentre (system.rs materialize_star, procgen::is_multiple) with component Star/Barycenter records and the stars' total mass; the planet search lists the components. Not decoded: SE's class tables and star physics (component temperatures, radii and classes from the port's main-sequence relations; remnant temperatures the port's), the split-decision streams.
- **06-008** (Procedural generation, fixed 2026-10-30): No comets in procedural systems: SE gives each single non-remnant star u%100+100 (100-199) comets on eccentric... Fix: SE: FUN_1404f9210 gives a single star u%100+100 comets (+0x1d40; 0 for brown dwarfs, lum code 7 letter >= 7, or system flag 0x8000). FUN_1404fc9b0: R = inner planet-zone edge (FUN_14050a890), q = tn_d(U(5,10)R, sigma (outer-R/4)/3, R/4, 2 outer - R/4), Q = tn_d(1.2 SOI, 0.8 SOI/3, 0.4 SOI, 2 SOI) (FUN_140441b80), e = clamp((Q-q)/(Q+q), 0, 0.9999), a = q/(1-e), inc/node/argp/M uniform +-pi; none if 2 outer - R/4 > 0.4 SOI. Name star + "C" + n. Body FUN_140502710 on seeds n*0x21ad+sys*0x2244 / sys*0xc49+n*0x1938: mass 10^U(-20,-8.6) MJ, density tn(500,100,200,3000) -> radius (FUN_1404c8990), albedo tn(0.04,0.01,0.01,0.1), rotation tn(2,1.5,0.1,20) h, tail values FUN_140486280; period with the comet mass. Ported: crates/se_core/src/procgen/se_comets.rs (generator, tail defaults, tail particles), se_layout keeps the comet count it already drew, mod.rs emits `Comet "<star>C<n>"` blocks (Mass, Radius, Albedo, rotation, AbsMagn, SlopeParam, CometTail, Orbit tilted like the planets). Checked: tests (100-199 comets per single star, q/Q ranges, tail defaults, parse into the catalog); Altair gets 157 comets (generated in 3 ms), AltairC119 at perihelion 2006-10-17 shows its tail (proc_altair_c119.png). Not ported: comets of multiple-star components (FUN_1404f9210:614-652, count from a pow() formula), the orbit stream state (the port seeds its own; SE uses whatever its thread RNG holds after the planets), and the composition/0x1210/0x1214 draws (FUN_1401aa500, FUN_14050a5f0, FUN_14050a520) that sit between the body draws on stream b.
- **07-001** (Camera and time, fixed 2026-10-28): Startup field of view: SE starts at FOV 45 deg (Controls FOV 45, DefaultFOV 45); port's camera and projection ... Fix: The camera starts at SE's 45 degrees (and the `fov` command defaults to it).
- **07-002** (Camera and time, fixed 2026-10-28): Time-rate steps: SE's L/K (and the toolbar buttons) step the rate through 1, 3, 10, 30, 100, 300... (two press... Fix: K/L and the top bar's << / >> step the rate through 1, 3, 10, 30, ... (`hud::rate_step`).
- **07-003** (Camera and time, fixed 2026-10-28): Go to arrival distance: SE arrives 4 radii from the centre (25,513 km for Earth, velocity set to 2,551 km/s, p... Fix: Go to arrives 4 radii from the centre (`GOTO_RADII`).
- **07-004** (Camera and time, fixed 2026-10-29): Go to and Center timing: SE's Go to accelerates and brakes by GotoSpeed 10, long flights last up to LongGotoTi... Fix: Go to lasts log10(distance ratio) + 1 s within CenterTime 1.5 .. LongGotoTime 20 s (GotoSpeed 10; SE's exact curve not decoded), G again restarts it as the 3 s fast Go to, Center turns in 1.5 s; test `goto_times`.
- **07-005** (Camera and time, fixed 2026-10-29): Land on object (Shift+G): SE flies down to the selected body's surface and binds to its rotation ("Land on obj... Fix: Land on object (Shift+G, navigation bar Land, dev command `land`): the camera drops to the ground (or sea) under where it started, logarithmically in altitude over SE's Go to timing, rotating with the body from the start (SyncRot), and turns to the level view along its heading; it ends 2 m up at 10 m/s. Assumed: the landing height and arrival velocity (SE's Land handler was not found in the exe).
- **07-006** (Camera and time, fixed 2026-10-29): Camera binding modes: SE has explicit Free / Follow / Rotate-with (SyncRot) binding to an object, chosen with ... Fix: SE's binding modes (FUN_140575f90 / FUN_14057a230 / FUN_14057a4a0 set BindMode 0/1/2 and nothing switches them by distance): Shift+D free, Shift+F follow, Shift+R rotate with the selection, navigation bar Free/Follow/Rotate buttons and "Following: X" / "Rotating: X"; Go to binds Follow, Land and a scripted Goto to a latitude/longitude bind SyncRot, script Follow/SyncRot/Free and location BindMode are kept (the port's automatic lock within 2 radii is gone); script trigger Control/SyncRot.
- **07-007** (Camera and time, fixed 2026-10-29): Auto horizon on planets: SE keeps the camera's horizon level near planets (Controls AutoHorizon true, toggled ... Fix: SE's auto horizon (FUN_140576090): with AutoHorizon on (key 5, saved as AutoHorizon), rotating with a body and lower than max(0.005 radii, its atmosphere's height), mouse and key turns keep yaw about the ellipsoid normal, pitch (stopping at 1.5698 rad) and roll as angles, so the horizon never tilts.
- **07-008** (Camera and time, fixed 2026-10-29): Moon and tidally locked bodies' rotation: SE turns the Moon with its "Moon-DE" rotation model (DE librations, ... Fix: Moon-DE from DE436's lunar librations (`JplEphemeris::lunar_euler`, 3-1-3 Euler angles), TidalLocked (longitude 0 at the parent, pole along the orbit normal) and SE's IgnoreMoonsRotationIAU (moons' IAU models give way to tidal locking), in `Ephemeris::orientation`; test `tidally_locked_moon_faces_its_planet`.
- **07-009** (Camera and time, fixed 2026-10-29): Time keys while paused and current time: in SE pressing L (or K) while paused resumes time ("The time will res... Fix: L and K resume paused time, Ctrl+\ (and the Now button) set the date to the system clock without touching the rate or the pause ("Current time").
- **07-010** (Camera and time, fixed 2026-10-29): Date/time entry: SE lets you click the date bar and type a date "YYYY.MM.DD HH:MM[:SS]" (Enter applies, "Inval... Fix: Date entry: click the date on the navigation bar and type "YYYY.MM.DD HH:MM[:SS]" (Enter applies, "Invalid date" on error).
- **07-011** (Camera and time, fixed 2026-10-29): Track object (T): SE keeps the camera turned toward the selection while you move ("Track object", "Tracking", ... Fix: Track object (T, navigation bar Track, "Tracking"; script Track/Untrack): the view stays turned to the selection while the camera moves.
- **07-012** (Camera and time, fixed 2026-10-29): Keyboard turning and roll feel: SE turns the camera with Numpad 4/6/8/2, rolls with Q/E or Numpad 7/9, moves a... Fix: SE's keyboard binds: Numpad 4/6/8/2 turn, Q/E or Numpad 7/9 roll, arrows and Numpad 1/0 also move, turns ramp in over TurnKeyAccel 0.2 s and out over TurnKeyDecel 0.5 s, Numpad 5 stops turning; letters with Shift/Ctrl/Alt no longer move the camera. Assumed: the turn rates (1 rad/s at 45 degrees, scaled with the FOV; roll 1.5 rad/s).
- **07-013** (Camera and time, fixed 2026-10-29): Selection keys and home address: SE's Backspace selects the previous object, Shift+H cycles the selection thro... Fix: Backspace selects the previous object (selection history), Shift+H cycles the home address (Earth, Sun, Solar System, Milky Way; SE's HomeWorld/HomeStar/HomeSystem/HomeGalaxy), Z stops the motion.
- **07-014** (Camera and time, fixed 2026-10-29): Saved places behaviour: SE's locations carry Vel (assigned velocity preset), BindMode, PhotoMode and Exposure,... Fix: Saved places keep SE's Vel (assigned velocity, pc/s) and BindMode, and the body the camera is bound to; going to a place sets the velocity, binds the camera to the place's body and no longer pauses time. Not done: PhotoMode and Exposure (the port has no photo mode switch).
- **07-016** (Camera and time, fixed 2026-10-29): Secondary camera commands: SE has Go to object's centre (Ctrl+G, into galaxies, nebulae, barycentres), Horizon... Fix: Ctrl+G goes to the selection's centre, End (script Horizon) levels the view to the selection's horizon in CenterTime, Ctrl+Numpad 5/4/6/8/2 look back/left/right/up/down, Numpad 5 (script StopRotation) stops turning, Home resets the zoom, middle click zooms to ZoomFOV 20 and back.
- **07-018** (Camera and time, fixed 2026-10-29): Controls settings: SE has a Controls settings tab with per-action Key 1 / Key 2 rebinding (seKeyBind, "<Press ... Fix: FOV range MinFOV 1 arcsec to MaxFOV 120 degrees, and PgUp/PgDn, Home and middle-click zooms animate over ZoomTime 0.25 s (logarithmically).
- **08-002** (Interface, fixed 2026-10-29): Interface skin: SE draws a skinned GUI (Skins.pak, 25 skins, default "Flat Blue": textured widgets from interf... Fix: SE's interface skins (`skin.rs`): the chosen `* skin.cfg` (Settings "Interface skin", saved as `Skin`, default Flat Blue, all 25 listed) sets egui's visuals from the colours its texture gives the dialog, button, slider and edit widgets (TextureHSL, WinColor*), its TextColor*, DialogShadowWidth, DialogHeaderCapital (window titles in capitals), ObjectInfoColor and Info*Color (info table); windows show the blurred frame behind them (gui.glsl BLUR_BACKGROUND: the bloom chain's blurred 1/8 level, tone-mapped, BackgroundBrightness of it added); SE's NotoSans font for English too. Not done: the textured 9-slice widget frames and the texture's toolbar icons (egui draws flat widgets), menu buttons (no main menu).
- **08-003** (Interface, fixed 2026-10-29): Toolbars: SE has four toolbars (Main left: icons opening every tool window; Filters right: object and overlay ... Fix: SE's four toolbars: main (tool windows) bottom left, filters on the right, navigation (date, rate, Go to/Center/Clear) and camera (nearest body, velocity, FOV, star magnitude limit) along the bottom; the top bar is gone. Not done: SE's icons and hints, auto-hide (Lock*Panel), exposure/photo mode and planet/galaxy magnitude limits.
- **08-004** (Interface, fixed 2026-10-28): Default overlay state: SE starts with orbits off (ShowOrbits false), labels off (LabelsMode 0), markers off, s... Fix: Orbits, labels, star names and the FPS readout start off as in SE; "Show FPS" in Settings (saved as `FPS`) or env `SE_SHOW_FPS=1` shows the frame rate.
- **08-005** (Interface, fixed 2026-10-29): Selected-object info table: SE prints the selection's info as on-screen text in the upper-left corner (ObjectI... Fix: The selection's info table is text in the upper-left corner (title coloured by kind), brief/full (filters toolbar "Full info", saved as TextMode), with the diameter.
- **08-006** (Interface, fixed 2026-10-29): Find object: SE's Find object is a dialog opened by F3 / the lens icon, and Enter only selects the object (tut... Fix: Find object is a dialog on F3 (and the main toolbar); Enter selects and closes it; the wiki moved to SE's I.
- **08-008** (Interface, fixed 2026-10-29): Solar system browser (F2) missing: grid of rendered, lit 3D thumbnails of the system's stars, planets and moon... Fix: Solar system browser (`sysbrowser.rs`; F2 at the selection's level, Ctrl+F2 at the camera's system, Alt+F2 at the root): a grid of ThumbnailSize 120 thumbnails lit from ViewAngle 35 degrees, log-scaled sizes (LogScaleBase 3) or RealSize, ShowDwarfPlanets / ShowDwarfMoons switches, planet barycentres shown as their primary with the companions as its satellites, asteroids and comets only where nothing else orbits; click selects, double click opens the satellites (or goes to the object), Up goes back, Center / Go to buttons. Partial: thumbnails are shaded spheres in the catalog colour (with atmosphere rim, rings, glowing stars), not SE's 3D renders.
- **08-009** (Interface, fixed 2026-10-29): Orbit line rendering: SE draws screen-space thick glowing lines (geometry shader, OrbitMajorThickness 5.0 px /... Fix: SE's orbit lines (orbit.glsl): a screen-space band 2 x OrbitMajorThickness (planets, moons, suns, barycentres, ships) or OrbitMinorThickness (dwarf bodies, asteroids, comets) wide with its exp(-27 u^2) profile and colour x2, the OrbitTrailsLength 0.72 trail (full just behind the body, dimmest just ahead), the path bent smoothly through the body's real position, hidden behind bodies' discs, fading in as the orbit grows past a few pixels; every system member has one, barycentres too (Earth's path round the Sun is the Earth-Moon barycentre's). Assumed: which kinds are "major", the size fade.
- **08-010** (Interface, fixed 2026-10-29): Labels: SE has LabelsMode off/brief/detail (cycled with '), per-type label switches (ships, planets, moons, dw... Fix: SE's LabelsMode 0/1/2 (key ' cycles off, brief, detail; brief leaves out minor bodies that look tiny, assumed), ShowMarkers (Ctrl+'), and the per-kind ShowOrbits*/ShowLabels*/ShowMarkers* switches with SE's defaults (Settings > Filter objects; saved; scripts set them by SE's names); labels for catalog galaxies, clusters and nebulae (brighter than magnitude 9 or wider than 0.6 degrees); markers MarkersSize 7 px; labels to the right (LabelsPos 2).
- **08-011** (Interface, fixed 2026-10-29): Label, marker and orbit-line colours: SE colours labels, markers and orbit lines per object type from the skin... Fix: Labels, markers and orbit lines take their kind's skin colour (`Info*Color`: planets, moons, dwarf planets and moons, asteroids, comets, suns, star/planet/asteroid barycentres, stars, clusters, nebulae, galaxies, spacecraft), the selected orbit `BodySelectedOrbitColor`; the info table's title uses the same kinds.
- **08-012** (Interface, fixed 2026-10-29): On-screen splash messages: SE shows a short centre-screen message after actions (ShowMessages true, MessageTim... Fix: SE's splash messages (`messages.rs`, ShowMessages, MessageTimeout 2 s, 0.35 s fade): "Time paused/resumed", "Time scale: xN", "Time reset", "Orbits/Markers/Selection pointer enabled/disabled", "Labels: brief/detail". Assumed: where on screen (a quarter down, centred) and SE's composition of the switch messages.
- **08-013** (Interface, fixed 2026-10-29): Startup splash and loading screens: SE shows a splash window while loading (banner.cfg: random splash00-19.jpg... Fix: Startup splash (`splash.rs`, banner.cfg): a random splash*.jpg at DesiredHeight 480 per 1080 px with the logo0990_1024 logo, progress bar and loading line, a random one of SE's quotes in the interface language (their keys from the translation files) and the copyright and version from SpaceEngine.exe's version resource; it covers the window for the first 3 s and fades. Not done: SE's separate splash window before the main one, a real loading log line, the in-session loading bar.
- **08-014** (Interface, fixed 2026-10-29): Object type and class strings: SE builds localized class descriptions (planet class words ordered and gendered... Fix: SE's type strings (`se_core::classname`): planet classes as FUN_1404d7c40 builds them from SE's classes (temperature, volatiles, bulk with size, "with life"; the locale's PlanetClassOrder*, Gender*, separators; TypeAtmoClass / style switches) e.g. "Planet (Temperate marine terra with life)"; star types as FUN_1401db610 (colour by letter O,B blue, A,F white, G yellow, K orange, M,L red; row by luminosity class; brown dwarf 0.012-0.0767 Msun, planemo below; white dwarf, neutron star, black hole, Wolf-Rayet, carbon/zirconium) e.g. "Yellow dwarf (G2V)"; galaxy, cluster and nebula types ("Spiral galaxy (Sb)"). Bodies outside the active system keep their catalog class. Not done: multiplicity words for star systems and asteroid orbit families.
- **08-015** (Interface, fixed 2026-10-29): Info table fields: SE's table shows diameter, class line, apparent/absolute magnitude, luminosity, spectrum, a... Fix: The table shows diameter, class, apparent and absolute magnitude, luminosity, temperature, mass, age, gravity, density, albedo, temperature, pressure, orbit, rotation, apparent size, height above the surface and lat/lon, RA/Dec. Not done: phase, insolation, spectrum line, azimuth/height.
- **08-016** (Interface, fixed 2026-10-29): Window and overlay hotkeys: SE uses F1/M universe map, F2 solar system browser, F3 find object, Shift+F3 star ... Fix: SE's keys: O orbits, ' labels, Ctrl+' markers, ; selection pointer, Ctrl+` hide the interface (was F2), F4 settings, Ctrl+F4 graphics, F7 locations, F8 or 0 chart (was F6), Shift+F3 star browser (F5 dropped). Not done: F1/M map, F2 system browser, X constellations and Ctrl+F11 (none of these exist yet); F1 keeps the port's controls list, M/N the music.
- **08-017** (Interface, fixed 2026-10-29): UI font: SE uses NotoSans-Regular with NotoSans-Medium for bold, falling back to NotoSansCJKsc then DejaVuSans... Fix: SE's font chain for every language: NotoSans-Regular, then NotoSansCJKsc-Regular and DejaVuSans for missing glyphs (CJK first for Chinese, Japanese and Korean), and the bold chain NotoSans-Medium / NotoSansCJKsc-Medium / DejaVuSans as the "bold" family used by window titles and the info table's object name.
- **08-020** (Interface, fixed 2026-10-30): Constellations: SE draws constellation lines, IAU boundaries and names (abbreviated or full, FullConstellation... Fix: SE: FUN_140261780 loads `catalogs/stars/Constellations.cfg` (FUN_140262d40: Zodiacal, LabelPos, Lines by star name before '/', Boundary RA h/Dec deg). Lines FUN_1402603f0 join the stars' 3D positions (modelview with translation), fade ConstelLinesFadeStart/End 4000-10000 pc from the Sun; boundaries FUN_14025fd90 at infinity (ConstelBordersAtInf) or a ConstelBordersRadius 10 pc sphere, names FUN_14025f7b0 at LabelPos at infinity, centred, both fade ConstelBordersFadeStart/End 200-800 pc. prim_line.glsl Gaussian exp(-r^2/LinesWidth), alpha F^2 s^2, colour F g rgb; seSmoothSwitch values squared (OverlayFadeIn/Out 0.2/0.35 s). Skin ConstelLines/Boundaries/LabelsColor (defaults 0.4 grey, 0,0,0.5, 0.5 grey, FUN_14036a560). Keys (FUN_1405ba8f0): X ToggleConstellations, Shift+X lines, Ctrl+X boundaries, Alt+X names. Script vars ConstLines/ConstBoundary/ConstLabel + <Abbr>/All/Zodiac (FUN_140263e60, FUN_140260970). So from another star the lines take other shapes while boundaries and names stay on Earth's sky until 200-800 pc; beyond 10 kpc nothing is left. Ported: `constellations.rs` (parser, IAU name table = locale keys, FullConstellationNames, drawing in egui over the scene with SE's profile and fades), switches/keys/fades/occlusion in `overlay.rs`, Filters toolbar button (right click: lines, boundaries, names, art, full names) and Settings entries in `hud.rs`, settings keys saved (settings.rs), Set/Get/SaveVars (scripting.rs). Lines and boundaries are cut by the ground/bodies (ellipsoid test). Checked: 89 constellations, 749 lines (2 with unknown stars); screenshots .audit/work/skygrid/orion.png (Earth), surface.png (cut at the horizon), betel.png (from Betelgeuse), far_500.png, far_last.png (7 kpc), far_c.png (40 kpc: none). CPU 0.45 ms with all parts on; off: no cost. Frame medians (interleaved runs, Earth view, FOV 70): baseline exe 5.8 ms, this build with overlays off 6.1-6.2, with constellations and all four grids on 7.0 (later runs of every build, baseline included, sat at a 16.7 ms cap from the shared GPU). Not ported: constellation art (SE 0.991 ships none; switch shown disabled), ExposureCalibration/Exposure factor on the colours (drawn after tone mapping), abbreviated Serpens labels shown as "Ser" (assumed). Lines are drawn over the stars (egui overlay), SE's layering is unverified.
- **08-021** (Interface, fixed 2026-10-30): Coordinate grids: SE draws equatorial, ecliptic and galactic celestial grids and a planet's geographic grid (S... Fix: SE: FUN_140266a90 draws the celestial grids when ShowGrids (master) and ShowGridEquatorial/Ecliptic/Galactic are on, each by FUN_140266db0 at infinity (rotation-only view) in its frame (FUN_140267b60: identity, obliquity about X, galactic), colour CelEquatorGridColor/CelEclipticGridColor/CelGalacticGridColor x ExposureCalibration/Exposure, alpha colour.a x grid^2 x master^2, prim_line.glsl width LinesWidth. Lines (FUN_140315270 primitives 9/10): parallels every 10 deg -80..+80, meridians pole to pole, 24 (1h) or 36 (10 deg). Labels: each meridian at lat -60..60 step 30 ("%ih" / "%i°"), every 3rd meridian's parallels ("%+i°"). Geographic grid (FUN_1404c7460): the 10-degree grid on each body's ellipsoid in its frame when its radius > 20 px, fading in to 40 px, GeographicGridColor x ShowGridGeographic^2, not under the master. Keys (FUN_1405ba8f0): Ctrl+; ToggleCelGrids (ShowGrids), Alt+; ToggleGeoGrids. Ported: `grids.rs` (geometry, labels, frames from se_core::coords, geographic grid on SystemBody orient/shape, front side only), switches/keys/fades in `overlay.rs`, Filters toolbar "Celestial grids" button (right click: the four grids) and Settings entries in `hud.rs`, keys saved and scriptable. Paths are culled by the view cone before projecting. Checked: grids.png (6h meridian through Betelgeuse at RA 5.9h Dec +7.4, ecliptic lat -16, galactic l 200 b -9 as expected; Earth's grid at right), geo.png (Earth from 15000 km), far_c.png (only the equatorial grid from 40 kpc). Unit tests: line/label counts, frames (equinox, obliquity, NGP). CPU 0.9 ms with all three grids and the geographic one on at FOV 70. Not ported: SE's exact label offsets (it moves labels by a FOV fraction; here a few pixels beside the point), the exposure factor on the colours, the night/VR colour override; the geographic grid is not depth-tested (drawn on the side facing the camera, over terrain).
- **09-001** (Graphics and settings, fixed 2026-10-29): Bloom: SE's bloom adds 0.5x (BloomBright) of a 6-level Gaussian mip pyramid of the whole frame (weights 0.50/0... Fix: SE's bloom replaces Bevy's (`bloom.rs`, `se_bloom.wesl`): six levels of 2x2 averages (frame clamped to 65535, bloom source luma to 50), each blurred with SE's 9-tap Gaussian (normalised), added at BloomBright 0.5 with SE's weights before tone mapping. EnableBloom, BloomBright and BloomThreshold are in the Graphics window and saved (`bloom::BloomSettings`); SE leaves ToggleBloom unbound.
- **09-002** (Graphics and settings, fixed 2026-10-29): Window and display settings: SE starts as a maximized window (Maximized true) and remembers its size and posit... Fix: Window and display (`display.rs`): the window opens maximized or at its saved size and place (SE's Display WindowSizeX/Y, WindowPosX/Y, Maximized, kept from winit while windowed), FullScreen 0 windowed / 1 fullscreen window / 2 exclusive fullscreen, VSync 0 off / 1 on / 2 auto (adaptive), Alt+Enter fullscreen, Shift+Enter next display, Ctrl+Enter minimize, Graphics window "Window mode" and "Vertical synchronization"; saved in the settings file's Display block. Not done: ScreenResX/Y, RefreshRate (exclusive fullscreen keeps the display's mode) and AspectRatio.
- **10-002** (Scripts, audio, spacecraft, fixed 2026-10-29): Music transitions and title: SE cross-fades tracks (MixDurationGUI 2 s, MixDurationLoop 5 s, MixDurationContex... Fix: Music as SE's Sound settings: consecutive tracks cross-fade over MixDurationLoop 5 s (the next starts 5 s before the end, from the Ogg length), N cross-fades over MixDurationGUI 2 s, M fades out over 2 s, each new track's title shows as an on-screen message (MusicShowTitle), default MusicVolume 20 of 100, saved as SE's 0..100 MusicVolume (Settings "Music volume"). Not done: SE's repeat/order modes and the context mode (left out with 10-001).
- **10-003** (Scripts, audio, spacecraft, fixed 2026-10-29): Interface sound effects: SE loads data/sounds/menu/button.ogg and menu.ogg (SDL_mixer) for GUI clicks and menu... Fix: Interface sounds (`sound.rs`): SE's data/sounds/menu/button.ogg on clicks of buttons, check boxes and list entries and menu.ogg when a drop-down opens (from egui's click events), at SoundVolume 30 of 100 (Settings "Sound effects", saved as SoundVolume). Assumed: which widgets sound. Not done: SetAudioDevice.
- **10-004** (Scripts, audio, spacecraft, fixed 2026-10-30): Ship sounds: every EngineEffect has a looping Sound and Volume (156 effects: Starship_engine, Starship_thruste... Fix: SE: FUN_1402a8ed0 reads each EngineEffect's `Sound`, `SoundVolume`/`Volume` (Volume wins); defaults: volume 1, thrusters 0.02; sound by group (FUN_1405693a0): Main/Retro `Shuttle_engine.ogg`, Hover `Shuttle_hover.ogg`, Thruster `Shuttle_thruster.ogg`, Hyper `Starship_warp.ogg`. Playback is SDL_mixer: `seSound::FadeIn/FadeOut` (FUN_140567f30/FUN_140568060, Mix_FadeInChannelTimed/Mix_FadeOutChannel), channel volume (FUN_140567eb0, Mix_Volume) at `SoundVolume`. No positional calls are imported (no Mix_SetPosition/SetDistance/SetPanning): no distance attenuation or panning. Ported: `EngineEffect::sound_file`, `sound_candidates`, thruster default 0.02 (se_core ship.rs); `PlumeSpec.sound/volume`; `ship/engine_sound.rs` plays one looping voice per sound file of the flown ship: volume = SoundVolume x min(1, sum of Volume x effect level), the level that sizes the plume (spooled engine, RCS/turn command, warp fraction), faded over 0.3 s; silent outside flight mode. Checked: unit tests (defaults, voice mixing); logs show "ship sounds: 3 voices" (Shuttle) and 2 (Dragonfly), files read from data/sounds/spacecraft. Not ported / assumed: SE's per-effect channels and its fade times for ship sounds were not traced (the code that calls FadeIn for engines was not found); summing effect volumes stands in for SDL_mixer's channel mixing.
- **10-013** (Scripts, audio, spacecraft, fixed 2026-10-30): Autopilot and targets: SE has ShipFlyTo, ShipWarpTo, ShipGoTo and ShipRotateTo ("Fly to target", "Warp to targ... Fix: SE keys: ShipSelectTarget T, ShipSelectRefBody P ("Nearest body" / "Influence body" / "Reference body"), ShipRotateTo C, ShipFlyTo Shift+G, ShipWarpTo Alt+G, ShipGoTo G ("Fly or warp to target"). Stages (locale): Preparing/Correcting jump vector, Preparing/Correcting fly vector, Accelerating, Turning for braking, Braking. FUN_140537060 runs the orientation program (constants above, see 10-014); FUN_14051f7b0 the warp program (boost so a jump never crosses the remaining distance in under two frames, "Correcting" when the velocity leaves the line); flags in `seSpacecraft+0xb8` choose main vs retro for braking (0x8000). Ported (`ship/autopilot.rs`): target = camera selection (T); reference body cycles nearest / strongest gravity / picked body (P) and sets velocity frame, altitude and holds; Rotate to (done < 0.1 deg); Fly to: turn to the line, accelerate, brake on `sqrt(2 a d)` (retro engines, or turn round first with the turn time allowed for), RCS cancels drift and gravity; Warp to: same with the drive on, physical speed capped, boost limited to half the remaining distance per frame, the warp multiplying velocity relative to the target; Go to warps when flying would take > 300 s. Stops at 4 target radii (the camera's Go to), then Sync velocity. HUD: target, distance, "Autopilot: Warp to target - Braking". Script: `flight target NAME/selected`, `flight ap flyto/warpto/goto/rotateto/off`, `flight ref nearest/influence/NAME`. Also fixed: the warp now multiplies the velocity relative to the reference frame (it multiplied the absolute velocity, Earth's orbital motion included). Checked: unit tests (point-mass Fly to and Warp to over Earth-Moon, no overshoot); runs: Dragonfly from 2000 km over Earth warps to the Moon, stops 6918 km from its centre (`warp_c.png`, log stages); Fly to the Moon from 10000 km (`fly_*.png`). Not ported: SE's fly program internals (FUN_14051efa0 hyperjump interpolation, stop distances, Landing stage), "Fly here"/"Warp here" (map), hyperdrive, target ships (TargetShipID). Guidance math is the port's.
- **10-014** (Scripts, audio, spacecraft, fixed 2026-10-30): Attitude holds: SE has Prograde, Retrograde, Radial, Antiradial, Normal Up/Down and Horizon (ShipHorRot) orien... Fix: SE keys (FUN_1405ba8f0 table): ShipPrograde 5, Retrograde 6, Radial 7, Antiradial 8, NormalUp 9, NormalDown 0, ShipHorRot End, ShipHoldAlt H, ShipFlightAssist U, ShipStopRotation Num5, ShipStopEngines Z, ShipStopWarpDrive Alt+Z, ShipSyncVelocity Shift+Z. Turning (FUN_140537060): bang-bang plan (FUN_140520e00, accelerate `sqrt(angle/TurnEngines)` then brake), tracking (FUN_1405211e0), re-plan beyond 5 deg, done below 0.1 deg; player torque ignored meanwhile. Kill rotation FUN_140539700 brakes along -w at TurnEngines. Ported (`ship/autopilot.rs`, `ship.rs`): the holds relative to the reference body (prograde = velocity in its frame, radial = away from it, normal = r x v, Horizon keeps heading and levels the wings), steering at `sqrt(2 a angle)` capped by the port's rate limit; Kill rotation, Hold altitude (vertical speed loop on hover/RCS, main engines when the nose is vertical), Sync velocity, Stop engines, Stop warp, Flight assist on U. HUD shows "Orientation: Prograde" etc.; script `flight hold MODE/killrot/sync/holdalt`. Key changes: T (was flight assist) is now Select target, H (was warp toggle) is Hold altitude, Z (was full throttle) is Stop engines; warp is Alt+= / Alt+- (or PageUp/PageDown, + engages), Alt+Z stops. F no longer leaves flight (it is RCS down). 0 and P are taken from browser.rs/stars.rs while flying (PreUpdate `take_flight_keys`), no edit there. Checked: unit tests (hold vectors, horizon levelling, steering convergence); `.audit/work/shipflight/aero_hold_a.png` (Horizon), `aero_hold.png` (Prograde, attack +0.1 deg). Not ported: Sync rotation (rotate with the body), SE's exact Hold altitude engine logic (FUN_140520260 drives main/retro in timed stages; not decoded).
- **10-017** (Scripts, audio, spacecraft, fixed 2026-10-30): Aerodynamics: designs carry Aerodynamics and AeroModel ("arrow", "shuttle"), the Physics key Aero true enables... Fix: SE: `ReadSchemeScript` (FUN_1405308b0) picks model 0 "none", 1 "shuttle" (`Aerodynamics true`), 2 "arrow" (`Aerodynamics false`), `AeroModel` overrides. Tables built by FUN_1401a3dd0: shuttle and arrow share one 91-angle x 11-coefficient table (DAT_14075c960, angles DAT_14075e8b0), cubic spline (FUN_140026010/FUN_140025ba0). Applied in FUN_14051b720 when Physics `Aero`, body has air and altitude < atmosphere top. "none" (FUN_14051d700): drag `5 q (L/10)^2 / m`. Shuttle/arrow (FUN_14051cde0): Aerosonde UAV constants x100 (S 0.55, b 2.8956, c 0.18994), lift (arrow `1.2 sin 2a`, shuttle table blended to `1.2 sin 2a + 0.01` over Mach 0.9-1.8), drag with transonic rise `(4 exp(-((M-1)/0.1)^2/2)+1) CD`, side force, force factor `min(0.55 q, 1e5)`, pitch/yaw/roll moments with inertia (800, 1755, 130), each clamped to 0.1 rad/s^2. SE's angular state is the negated physical rate (quaternion update `dq*q`, q world->ship). Ported: `se_core/src/ship/aero.rs` (+ `aero_table.rs` dumped from the exe), `ShipDesign::aero`; `se_engine/src/ship.rs` applies forces per sub-step and moments per frame, `ship/air.rs` gives each body's air (climate completion or catalog Atmosphere; polytropic below 0.2 atm, isothermal above). HUD: air speed, Mach, dynamic pressure, attack/sideslip, aero accel, density, speed of sound. Ships window: "Aerodynamics" checkbox (Physics Aero), `flight aero on/off`. Checked: unit tests (spline nodes, drag direction, lift and weathervane signs, Mach rise, force cap, Earth density 1.23); `.audit/work/shipflight/aero1.png` (Shuttle Mach 1.58, 117 kPa), `aero_hold_a.png` / `aero_hold.png` (Horizon then Prograde hold, Mach 0.45-2.0). Not ported: aircraft mode and control surfaces (ShipElevator/Rudder/Ailerons/Flaps axes, limits/rates are parsed by SE but have no default keys); deflections are 0. FUN_140259df0's exact upper-atmosphere temperature, SE's "ship destroyed above 500 m/s^2 unless God" and the 10 kPa "high pressure" flag.

## Refuted by the checkers

Reported by an auditor, then disproved; listed so they are not re-reported.

- **01-024** double stars from outside their system: the port draws every component of a multiple system at the barycentre position in the starfield, so ... Reason: No SE evidence that SE splits distant multiples in the starfield: its star reader (FUN_140475fb0) puts each StarBarycenter in the star octree as one record (the "patching StarBarycenter ... unwanted star in the binary barycenter" warning confirms one point per barycentre), and the components (planets/Stars-bin.sc Albireo Aa/Ac/B) are planet-catalog bodies loaded with the system, which is what the port does; whether SE loads more than the nearest system (CacheSizeSolarSystems 64) is unproven and would be a different gap
