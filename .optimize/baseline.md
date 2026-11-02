# Baseline (2026-11-02, before any optimization)

Commit `24c82fb` (V1 + the 55 bug fixes) plus the uncommitted frame-time logger
(`crates/se_engine/src/perf.rs`, a `perf` dev command in `camera.rs`, one line in `main.rs`; it does
nothing unless `SE_PERF` is set). Release build (`lto = "thin"`, `codegen-units = 1`).
Machine: NVIDIA RTX 5080 (driver 610.88), AMD Ryzen 9 7900X (12 cores), 31 GB RAM, Windows 11.
Window 1920x1080 (the user's saved window), vsync off (`SE_NO_VSYNC`), music off.

Scenes: `.optimize/bench/all.sh` (commands there). Date fixed at 2026-06-21 12:00 UTC.
Runs: `base1` and `base2` (`.optimize/scratch/base1`, `base2`). Frame times in ms.

| Scene / segment | What | base1 mean | base2 mean | median | p99 | max | 1% low fps | entities | peak MB |
|---|---|---|---|---|---|---|---|---|---|
| startup | first 300 frames after launch | 14.45 | 18.91 | 9.3-15.5 | 89-94 | 803-833 | 2-3 | 4874 | 1756 |
| space_earth | 20,000 km from Earth | 9.12 | 8.32 | 8.25 | 10.8-12.3 | 12-13 | 78-87 | 5538 | 1794 |
| galaxy_view | Milky Way from 30 kpc | 6.50 | 6.15 | 6.06 | 9.4-9.5 | 11 | 99-101 | 1158 | 1698 |
| fly_1e14 | flying through stars, 1e14 km/s | 8.33 | 15.67 | 8.0-16.5 | 13-21 | 30-33 | 40-53 | 2229 | 1779 |
| fly_1e15 | flying through stars, 1e15 km/s | 12.07 | 16.94 | 8.9-16.4 | 39-46 | 54-63 | 19-22 | 2538 | |
| earth_still | Earth, 300 m up, still | 15.17 | 14.98 | 14.9 | 17.1-17.3 | 19 | 55-56 | 10983 | 1760 |
| earth_look | Earth, turning 0.36 deg/frame | 15.01 | 14.73 | 14.6 | 17.2-17.6 | 19-20 | 55-56 | 10985 | |
| mars_still | Mars, 300 m up | 9.98 | 10.18 | 9.8-10.1 | 12.5 | 14-16 | 70-74 | 12387 | 2147 |
| mars_look | Mars, turning | 10.65 | 10.55 | 9.8-10.4 | 13.5-18 | 15-23 | 51-70 | 12396 | |
| rs_still | RS 8514-1828-5-27938-309 2, 300 m up | 11.10 | 11.87 | 11.0-11.7 | 12.3-15.8 | 13-34 | 45-79 | 5138 | 1879 |
| rs_look | same, turning 0.9 deg/frame | 11.69 | 11.69 | 11.5-11.7 | 14-15 | 16-33 | 59-66 | 5138 | |
| rsg_still | same planet, 2 m up | 13.87 | 13.59 | 13.3-13.6 | 15.7-17.2 | 53-57 | 32-33 | 6027 | 1996 |
| rsg_look | 2 m up, turning 0.9 deg/frame | 14.61 | 14.41 | 14.3-14.6 | 17.0-17.7 | 49.3-49.5 | 45-47 | 6027 | 2003 |
| neb_still | inside the Skylias Stellar Nursery | 16.68 | **5.99** | 6.1-16.7 | 7.3-18.3 | 7.6-21 | 49-133 | | 1892-1930 |
| neb_look | same, turning | 12.86 | **5.26** | 5.2-16.2 | 6.7-18.8 | 10-25 | 48-122 | | |
| tar_still | inside the Tarantula Nebula | 5.32 | 5.84 | 5.4-5.8 | 6.4-8.1 | 8.7-9.5 | 115-130 | | 1755-1774 |
| tar_look | same, turning | 5.52 | 5.67 | 5.5-5.7 | 6.9-7.4 | 8.6-8.7 | 124-134 | | |

**60 fps cap from outside the port:** base1's nebula (median 16.66 ms) and base2's fly scene (median
16.4-16.5 ms) were capped at 60 fps by something outside the port. The GPU work was only ~5 ms, the
port has no limiter, vsync was off and the monitor is 239 Hz. Most likely the window wasn't in focus
(Windows doesn't give a newly launched window focus while the user is busy in another app, and the
NVIDIA driver can cap background apps). Any segment whose median sits at ~16.6 ms is suspect and must
be rerun with the port window in front. The true values: nebula ~6.0 ms (base2), fly_1e14 ~8.0 ms and
fly_1e15 ~8.9 ms median (base1). The surface scenes repeat within ~0.3 ms. Compare medians of
several runs.

## GPU (timed render passes, ms per frame, base1)
Only Bevy's own passes have timestamps; the port's bloom, colour correction, upscale and egui passes
don't. Earth: opaque 4.0, transparent 0.7, uniform_allocation 0.9, bin_unpacking 0.9. RS ground:
opaque 1.1-2.3, transparent 0.2-1.5. Nebula: transparent 5.25 (frame 16.7 with ~6 ms of CPU work:
GPU-bound). Tarantula: transparent 4.5. Space/fly/galaxy: transparent 0.6-1.2.

## CPU profile (trace build, `.optimize/scratch/trace/*.full.txt`)
Bevy runs rendering on its own thread; a frame takes about max(main app, render app).
- **RS ground, looking around:** render app ~19 ms, main app ~10 ms. Render side: Bevy's
  per-entity mesh/material preparation (instance buffers 2.75, specialization 2.1, preprocess bind
  groups 2.0, queueing 1.5, command submission 3.6). Main side: terrain 1.1, star field streaming
  0.7, nebula volumes 0.6, ~20 material types' specialization checks 0.15-0.24 each.
- **Earth surface:** render app ~17 ms; info panel (draw_hud) 3.4 ms; moving the Solar System's
  bodies 3.3 ms; terrain 1.6 ms.
- **Flying through stars (1e15):** main app ~20 ms: select_active_system 9.5 ms average (46 ms
  worst), spawning system members 1.4, black-hole lens 1.8, aurora spawning spikes 18.6 ms, mesh
  upload spikes 15.7 ms.
- **Nebula:** CPU light (~6 ms); GPU-bound.

## Reference screenshots
`.optimize/bench/refs.sh` (time paused, fixed date) into `.optimize/scratch/refA` and `refB`;
`compare.py refA refB` gives the run-to-run noise floor for the identical-output check:
Earth, Mars and both RS planet shots are pixel-identical between runs (PSNR 96-107 dB, no pixel
off by more than 8); galaxy, Saturn, space, stars and the nebulae differ slightly (mean 0.02-0.43
of 255, 0.01-0.3% of pixels off by more than 8; PSNR 43-54 dB), from things that move between runs.

## Tests
`cargo test -p se_core` and `cargo test -p se_engine --release`: results in
`.optimize/scratch/tests_base.txt`: se_core 301 passed (20 ignored), se_engine 184 + 3 passed
(2 ignored). The release build has no warnings.
