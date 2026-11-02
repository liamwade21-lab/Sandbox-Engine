# Bug hunt report (2026-11-01)

Scope: the whole port (`crates/`, ~92k lines of Rust and WESL), level **L**: 11 hunters (one per
subsystem ticket in `tickets/`), 13 verifiers, 4 fixers. Raw findings are in `findings/`, the
verifiers' verdicts in `verdicts/`, hunters' repros in `scratch/`.

64 findings reported → 62 held up (53 confirmed, 9 likely), 2 rejected. Severities below are the
verifiers' corrected ones.

## High

| ID | Bug | Where | Trigger → impact | Status |
|---|---|---|---|---|
| 07-1 | ~360 catalog stars drawn twice (nameless ghost copies, brightness doubled) | se_core/catalog.rs:583-596 | .sc star or barycentre sharing a non-first name with a HIPPARCOS star (Ran/EPS Eri, Polaris, UPS And) → two stars, often parsecs apart | confirmed |
| 08-2 | Tutorial 1 always gets stuck at the "follow" step | scripting.rs:1919 | `WaitTrigger "Control\|Follow"` after the earlier Go to already bound the camera → can never pass, only Skip helps | confirmed |
| 09-1 | Ship is left behind by its planet above 10x time | ship.rs:1386 | ship time capped at 10x while planets move at full rate → planet races away at ~30 km/s × rate | confirmed |
| 09-2 | Ship drifts while paused; Sync velocity cancels orbital speed | ship.rs:473, 521, 1386 | pause while flying → ship moves, bodies frozen, body velocity read as 0 → planet pulls away at 30 km/s after unpause | confirmed |
| 09-3 | Ship lands on an invisible sphere above the ground of flattened planets | ship.rs:1971-1977 and others | Earth/Mars/Saturn: ship contact uses a sphere, terrain is drawn on the ellipsoid → "Landed" 10 km above Mars at 60N | confirmed |
| 09-4 | Parked and landed ships get lost after visiting another system | ship.rs:1883-1910 | Parked/Landed hold entity ids that die on a system switch → ship left in space or inside the planet | confirmed |
| 10-1 | Wide hi-res screenshots lose ground/sea in the outer tiles | screenshot.rs:706, capture.rs:150 | Bevy culls each tile with a centred frustum → outer columns show only sky (default 8000x4000 too) | confirmed |
| 02-1 | Catalog planets with no Class are drawn as rocky | se_core/surface_gen.rs:1981-1987 | HD 189733 b (hot Jupiter) shows as "megaterra", plain blue disc; 1360 bodies | confirmed |
| 02-2 | Equilibrium temperature ~9.5% too high (not SE's formula) | se_core/climate.rs:2128 | every body with an atmosphere block or procedural atmosphere: Earth 299 K vs 276 K; changes snow, ice caps, climate | confirmed |
| 05-2 | All stars in far galaxies get the same system | se_core/procgen/mod.rs:422 | galaxies past 2.147 Gpc on all axes (18.6% of the universe): seed saturates → identical systems | confirmed |

## Medium

| ID | Bug | Where | Status |
|---|---|---|---|
| 03-1 | Continue loses the selection/binding on procedural objects (camera then drifts off the planet) | session.rs:135, ship/persist.rs:257 | confirmed |
| 03-3 | Free flight keeps going (and turning) while typing in a text box | camera.rs:630 | confirmed |
| 03-4 | Two-button zoom jumps to the edge of a galaxy/cluster you are inside | camera.rs:592-595 | confirmed |
| 04-1 | Mercury's prime meridian off by ~247° (diurnal terms read in 3s, rows have 6) | se_core/rotation.rs:98-104 | confirmed |
| 04-2 | Parabolic comets move √2 too fast | se_core/orbit.rs:153, 184 | confirmed |
| 04-3 | Clicked/searched procedural systems are never unloaded | system.rs:507-565 | confirmed |
| 04-4 | Moons without RefPlane use the ecliptic, SE uses the equator (Proteus 28° off) | se_core/orbit.rs:94-99 | confirmed (Moon part) |
| 06-1 | Galaxies missing from a procedural galaxy node built while far away | procgalaxies.rs:215-219 | confirmed |
| 06-2 | Star search for giants skips the level holding ~87% of them | star_search.rs:464 | confirmed |
| 06-3 | System search shows 0 planets for unvisited catalog stars | star_search.rs:397-404 | confirmed |
| 07-2 | 32 hand-made galaxies overwritten by NGC-IC.csv rows (wrong type, distance, size) | se_core/catalog.rs:397-406 | confirmed |
| 08-1 | Script `if` line with a non-ASCII character crashes the port | se_core/se_script.rs:453 | confirmed |
| 08-3 | Tutorial "go to" step can stick if a camera move is running when it starts | scripting.rs:1825 | likely |
| 08-4 | Unbalanced if/loop makes a script restart forever (log grows fast) | se_core/se_script.rs:599-608 | confirmed |
| 08-6 | Closing the app mid-tutorial saves the tutorial's settings over yours | settings.rs:535, scripting.rs:551 | confirmed |
| 09-6 | Terrain code overwrites the ship's near plane → hull clipped | terrain.rs:2609 | confirmed |
| 10-2 | Window grows each launch on 125%/150% displays | display.rs:89, 184 | confirmed |
| 02-3 | Small mini-Neptunes get rocky terrain from zero parameters and lose clouds | se_core/proc_surface.rs:99-120 | confirmed |
| 05-1 | Revisited multiple-star systems regenerate differently (root mass overwritten) | system.rs:840 | confirmed |
| 05-3 | Binary planets' extra moons orbit in the ecliptic, not the system plane | se_core/procgen/mod.rs:1603-1620 | confirmed |
| 11-1 | Black-hole lensing and disk vanish beyond 2 pc | blackholes.rs:666 | confirmed |
| 11-2 | Aurora with no radius → billions of particles → out-of-memory crash (add-on catalogs) | aurora.rs:230 | confirmed |
| 01-1 | Far sea drawn as if underwater on flattened planets, camera low | shaders/planet.wesl:168-170 | confirmed |

## Low

03-2 Go to from inside 4 radii jumps 3 radii on frame one (camera.rs:1125) · 03-5 frame-time mode
clock drift (universe.rs:173) · 06-4 search planet click fails after the system was unloaded
(star_search.rs:759) · 06-5 galaxy at index 8514 shares the Milky Way's name code
(procsystems.rs:289) · 07-3 13 massless barycentre members never move (catalog.rs:1087) · 07-4
places.cfg not written atomically (data.rs:53) · 08-5 multi-line `if {` condition parsed as "non-zero"
(se_script.rs:439) · 08-7 non-Vorbis music file crashes (music.rs:290) · 08-8 script `FOV` resets to
60° not 45° (scripting.rs:60) · 09-5 finishing Rotate-to switches off manual warp (ship.rs:1470) ·
09-7 landed ship re-reads coarse ground and can fall (likely; ship.rs:1627) · 10-3 HD screenshot's
LandLOD saved if you quit mid-job (screenshot.rs:685) · 10-5 named script screenshots overwrite each
other (screenshot.rs:132) · 02-4 iron/carbon worlds with oceans bury the ocean (likely;
proc_surface.rs:125) · 02-5 surface generator ignores some planets' tilt (likely; surface_gen.rs:2008)
· 05-4 rogue planets get metallicity in the wrong units (procgen/mod.rs:341) · 05-5 cluster records
kept across a Milky Way rebuild (likely; procsystems.rs:590) · 11-3 add-on atmosphere "still
generating" treated as missing (system.rs:740) · 11-4 geographic grid fades with fade⁴ not fade²
(grids.rs:174) · 01-3 terrain morph distance baked at build (terrain.rs:1976) · 01-4 cube-edge normal
seam (terrain.rs:1346) · 01-5 generated grids kept longer than needed (terrain.rs:2213) · 01-6
orphaned generator results (terrain_gen.rs:131).

Rejected: 10-4 (SE also overwrites skybox exports), 01-2 (texture cache already listed in
missing_features.md, bounded in practice).

## Will fix

All 10 high, these medium: 03-1, 03-3, 03-4, 04-1, 04-2, 04-3, 04-4 (Moon/Star part), 06-1, 06-2,
07-2, 08-1, 08-3, 08-4, 08-6, 09-6, 10-2, 02-3, 05-1, 05-3, 11-1, 11-2, 01-1, and these cheap low
ones: 03-2, 06-4, 07-4, 08-5, 08-7, 08-8, 09-5, 10-3, 10-5, 02-4, 05-4, 11-4, 01-3, 01-4
(generated worlds).

## Won't fix now

All of these except 01-5 were fixed afterwards in round 2, at the user's request (see "Round 2"
below).

| ID | Why |
|---|---|
| 06-3 | Needs every unvisited catalog star in a search to generate its system: a performance/design call. |
| 06-5 | The port's galaxy numbering differs from SE's; renaming galaxy codes changes object names, needs a design decision. |
| 07-3 | SE's rule for massless bodies isn't known; guessing a density would be invented behaviour. |
| 09-7 | Needs a design for how a left-behind landed ship keeps its height. |
| 02-5 | Needs SE's tilt-to-orbit computation ported; the hunter's suggested fix would make it worse. |
| 05-5 | Rare (galaxies off at start, or a slow load) and needs a design for re-pointing promoted records. |
| 11-3 | Only add-on atmosphere models; needs a new retry system. |
| 03-5 | Only in frame-time mode (off by default), 0.06%; anchoring touches every place that sets the date. |
| 01-5 | Bounded memory, and it sits right next to the just-fixed grid-cache bug; not worth the risk. |
| 01-6 | A few MB per system switch. |

## Fixes

All 46 "will fix" items were fixed, one partly: 04-4 changes the default for moons only, not
stars. Four fixers worked in separate worktrees. Their patches (`F1.patch` to `F4.patch` in this
folder) were applied to `main`'s working tree. Nothing is committed.

| Fixer | Items | Files | Tests added |
|---|---|---|---|
| F1 | 07-1, 07-2, 04-1, 04-2, 04-4 (moons only), 08-1, 08-4, 08-5, 07-4, 11-1, 11-2 | catalog.rs, rotation.rs, orbit.rs, sc.rs, se_script.rs, data.rs, blackholes.rs, aurora.rs, docs/blackholes.md | sc_stars_merge_csv_stars_by_any_name, sc_galaxies_replace_csv_rows_by_any_name, moons_default_to_the_parent_equator, diurnal_terms_read_six_column_rows, parabolic_orbit_moves_at_escape_speed, conditions_with_non_ascii_comments, multi_line_conditions, unbalanced_if_warns (extended), lens_cutoff_follows_screen_size, ovals_without_a_radius_keep_the_particle_budget |
| F2 | 02-1, 02-2, 02-3, 02-4, 05-1, 05-2, 05-3, 05-4, 04-3, 06-1, 06-2, 06-4 | climate.rs, climate/display.rs, climate/tests.rs, surface_gen.rs, proc_surface.rs, procgen/mod.rs, system.rs, star_search.rs, procgalaxies.rs | classless_catalog_giants_are_classed_by_size, equilibrium_temperature_follows_se, se_giant_bulk_makes_a_giant_kind, solid_worlds_with_an_ocean_have_a_sea_level, position_seeds_wrap_far_from_the_origin, binary_planet_moons_turn_into_the_ecliptic_frame, unloaded_multiple_systems_generate_the_same_again, giant_search_scans_the_levels_giants_are_in |
| F3 | 08-2, 08-3, 08-6, 08-7, 08-8, 03-1 (session), 03-2, 03-3, 03-4, 10-1, 10-2, 10-3, 10-5, 11-4 | camera.rs, scripting.rs, session.rs, settings.rs, music.rs, screenshot.rs, capture.rs, display.rs, grids.rs | travels_and_binds_are_counted, settings_held_while_a_script_will_give_them_back, non_vorbis_bytes_are_rejected, a_procedural_ref_body_is_bound_once_it_exists, zoom_floor_only_for_solid_bodies, goto_from_inside_the_arrival_distance_does_not_jump, sub_view_tiles_cull_with_their_own_part, oversized_window_fits_its_monitor, named_shots_count_up_per_prefix |
| F4 | 09-1, 09-2, 09-3, 09-4, 09-5, 09-6, 03-1 (ship), 01-1, 01-3, 01-4 (generated worlds) | ship.rs, ship/persist.rs, terrain.rs, planet.wesl | fast_time_and_pause_keep_the_ship_with_its_body, carry_turns_with_the_body_only_close_to_it, ground_lies_on_the_ellipsoid |

Checked in the port as well as by tests:
- **07-1:** no csv star is left drawn under a name that now belongs to a .sc star (was 360). The
  named stars (Sun, ALF Cen, Polaris, Sirius, Vega…) resolve as before.
- **07-2:** NGC 4079 is back to Sa, 95.3 Mpc, and NGC 7318A/B appear once each.
- **10-1:** two 2048x512 Earth-surface mosaics came out seamless.
- **09-1:** at 1000x the ship stays 19,999 km from Earth (was 175,588 km).
- **09-2:** paused, it stays at 19,998 km.
- **09-4:** a parked ship is back at 20,000 km after a Sirius trip (was 42,052 km).

Decisions the fixers made:
- **04-4:** star orbits keep the ecliptic default. Nothing shows what SE uses as a barycentre's
  equator, and changing it would turn about 4,000 orbits.
- **07-2:** NGC-IC_fixed.csv is still loaded (75 galaxies). The SE log that lists only NGC-IC.csv is
  older than the current catalog pak, so it can't settle the question.
- **06-1:** procedural galaxy meshes now hold every galaxy in a node, about 15% more sprites. The
  shader hides the faint ones. Worth a look at frame rate.


### What's left
- Everything under "Won't fix now" above.
- A parked or landed ship whose generated system is freed from the cache (more than 64 systems
  visited) still loses its place. `system.rs` would need to keep ships' systems loaded.
- Optional (F2): `terrain.rs` around line 764 could also call `with_se` for bodies with a bump
  map, for the rare bump-mapped body that SE classes as a giant.
- Optional (F1): `orbit.rs:136-139` gives mean anomaly 0 for a `PericenterEpoch` with no
  `Period`. No stock catalog uses that key.
- `hud.rs:168`, the info panel's "elevation", still measures from the reference sphere. That may
  be SE's own value.
- No port run yet for 01-1 (sea seen from underwater) or 01-4 (cube-edge normals).
- **Not a bug (checked afterwards):** the info panel's Lat/Lon looked flipped (−44.90° for a
  camera placed at +45°) in daytime screenshots. Logging the panel's numbers and boosting the
  crop's contrast showed it reads +44.90° everywhere it should. The faint "+" just loses its
  vertical stroke against the bright sky. The terrain under the camera (desert at 45°N 90°E, forest
  at 45°N 135°E) matches too. (The 45 → 44.90 difference is geocentric vs. direction latitude on
  the flattened Earth.)

### Build and test results on the merged tree
- `cargo test -p se_core`: 296 passed, 19 ignored. With `--ignored` (reads the install): 19
  passed.
- `cargo test -p se_engine --release`: 178 + 2 passed, 2 ignored.
- `cargo build --release`: finished in 7m 19s with no warnings. `target\release\spaceengine-rs.exe`
  is up to date.
- Smoke runs on Earth's surface (night and day): no panics.

## Round 2: the "won't fix now" items

The user asked for 9 of the 10 to be fixed; 01-5 stays as it is. Four fixers (G1-G4,
`fixers/G*.md`) worked in worktrees on top of the round-1 fixes. Their changed files were
copied into `main`'s working tree. Nothing is committed.

| ID | What was done | Files | Test |
|---|---|---|---|
| 06-3 | Catalog stars with no catalog planets now get the system SE makes on arrival (same generator and seed as system.rs), built on a few threads. Clicking such a match selects the star. `search 50 life`: 15.1 s → 17.6 s, with 653 more stars getting systems. | star_search.rs, system.rs | catalog_stars_without_planets_list_the_system_made_on_arrival |
| 11-3 | `get_or_load` returns Ready / Pending / Missing. A pending add-on model is attached when it's ready. The body's ground waits for it, so it's built with the atmosphere. Stock models are unchanged. Auroras also wait for it (aurora.rs, small extra change). | atmosphere.rs, system.rs, aurora.rs | a_model_still_being_generated_is_pending_not_missing |
| 06-5 | Galaxy name codes swap: the Milky Way is 8514, and NGC 7185 (port index 8514) takes the Milky Way's port index. Milky Way names are unchanged. | proc_galsys.rs, catalog.rs, procsystems.rs, clusters.rs | galaxy_codes_do_not_clash_with_the_milky_way, galaxy_codes_swap_with_the_milky_way |
| 05-5 | On a Milky Way cluster/nebula rebuild, the records already promoted (and their member stars) are retired: names freed, not drawn. A selection on one is cleared. | catalog.rs, procsystems.rs | retired_nebula_frees_its_name |
| 07-3 | SE's rule (FUN_140503b60): a catalog asteroid with no mass gets 3000 kg/m³ and a comet 500 kg/m³. A barycentre with no stars weighs its bodies. The 13 frozen bodies now all orbit (Florence I: a = 6.4 km, P = 8 h). | catalog.rs | massless_small_bodies_and_body_barycenters_move |
| 02-5 | The generator's tilt is now SE's (FUN_140515e50): the spin axis measured against the orbit normal of the first ancestor orbiting a star or barycentre. Locked bodies are 0 (Proxima b, TRAPPIST-1 e). Mars 25.2°, Titan 28.0°. **Earth now gives 20.9°**, measured against its orbit around the Earth-Moon barycentre, as SE's code does (not checked in SE itself). | surface_gen.rs, climate.rs, climate/tests.rs | tropic_latitude_follows_the_tilt_against_the_orbit |
| 03-5 | Frame-time mode keeps a start date plus simulated seconds; it restarts when the date or rate is changed elsewhere. Real-time mode is unchanged. | universe.rs | frame_time_clock_does_not_drift |
| 09-7 | A landed ship remembers how fine the ground it landed on was, and only ground at least as fine moves it or lets it go. A landed ship is now saved landed, at its spot, and restored there. | ship.rs, ship/persist.rs, terrain.rs | landed_ship_ignores_coarser_ground, spacecraft_round_trip (extended) |
| 01-6 | `TerrainGen::forget` drops a removed Terrain's in-flight grid results, now and when they arrive. | terrain_gen.rs, terrain.rs | forgotten_jobs_leave_no_grids |

Notes from the fixers:
- **02-5:** the port still *draws* completed bodies that have no tilt given at 0° to the ecliptic,
  while the generator now uses the completion's tilt. Making the drawn axis match would be a
  separate change in the catalog/ephemeris code.
- **09-7:** seen from far away, a landed ship can sit inside coarse ground until finer terrain
  loads.
- **06-3:** the optional faster search would skip comets for catalog stars, as procedural rows
  already do. It was left out, since it's only safe if life doesn't depend on comets.

## After the hunt: the blue gas giant (user's video, 2026-11-02)

**Bug:** approaching the superjupiter RS 0-2-14-2255-21938-0-0-371 1 (it spins every 10 h, so it
is strongly flattened), most of it was flat blue. Only a band showed cloud bands, and up close
the edge between them flickered in triangles that changed as the camera moved. From afar, a waffle
grid covered the disc.

**Cause:** both faults are in the haze (aerial perspective) step of `planet.wesl` `finish`.
- **The flat blue:** the camera and each ground point were divided by the planet's equatorial
  radius before the haze tables were read. On a flattened body the ground near the poles then lay
  inside the tables' ground sphere. The haze cut it to almost nothing and left only blue
  in-scatter. At the equator the ground sat right on that sphere, so rounding flipped each spot
  between the two looks: the flickering triangles.
- **The waffle:** between its corners each flat mesh triangle sags a little inside the sphere
  (about 0.4 km on this giant from afar), so the haze darkened every triangle's middle.

**Fix (shader only):**
- The haze now works in the body's own frame with the ellipsoid undone, like the sky shell
  (`table_local`, `body_axes`, a new `axis_len` varying).
- Each ground point is lifted to the ground's radius at its own height, never lowered.

**Checked in the port:**
- On that giant, from 500,000 km and 180,000 km, the bands cover the whole disc with no flat
  area and no grid.
- Earth from orbit, at sea and in the Alps, Mars, Jupiter 2 km up and Saturn all look as before.
- No shader errors.

There is no automatic test: the project has no shader tests. Debug passes (painting branches and
quadrants, switching passes off) found the haze step to be the cause; see `scratch/giant/`.

## After the hunt: flickering auroras (user's video, 2026-11-02)

**Bug:** auroras flickered: from one frame to the next they jumped between bright and almost gone
(seen on RS 0-2-14-2255-21938-0-0-371 2).

**Cause:** `aurora.rs` worked out the flash phase (ScaleTime.w) from seconds (`jd x 86400`). SE
uses its clock in days since J2000. FUN_140451fe0 turns the 128-bit clock into days, which
FUN_140270900 then multiplies by 86400 for seconds, and FUN_1405d30b0 sets the epoch to JD
2451545. So the phase ran 86400 times too fast. At 60 fps the noise it slides (11 texels of a
256-texel random texture per frame) gave every frame a new random brightness.

**Fix:** a new `flash_phase(flash_freq, jd)` uses days since J2000. Test `flash_phase_moves_slowly`
checks that the phase is 0 at J2000, moves FlashFreq/10 a day, and slides the noise by under 0.01
texel per frame (the old code moved it 11 texels).

**Checked in the port:** the video's spot (52.9 N 49.3 E, 901 km up), six consecutive frames:
the aurora's mean brightness is the same in every frame (13.07). Tests: se_engine 184 + 3
passed. Release built.
