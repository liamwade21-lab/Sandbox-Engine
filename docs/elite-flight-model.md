# Elite Dangerous's flight model, as the Sidewinder mod uses it

Each rule of the ship's flight in our own words, with its constants, grade and source (`docs/spec-mods-sidewinder.md`,
"What byte for byte honestly means here"; the spec's C17). Grade **C** rules were read out of `EliteDangerous64.exe`
(version 2026.10.23.334925) by offline study with Ghidra (the user's D3); the address is where the rule was found.
Decompiled code is not kept in the repository (`re/elite/ghidra/`, out of git): only these formulas, in our words.

**The ship's own numbers are not in the exe.** The rules read a "performance block" of 51 values that the game loads at run
time (believed to come from the encrypted data files). Where those numbers come from in the mod:

| Block field | Meaning | Value for the stock Sidewinder | Source and grade |
|---|---|---|---|
| 0x00, 0x0C | top speed, boost speed (m/s) | 220, 320 | B (coriolis `speed`, `boost`; EDSY agrees) |
| 0x38..0x40 | turn rates (deg/s) | pitch 42, yaw 16, roll 110 | B (coriolis). EDSY says pitch 43, yaw 17: **conflict**, to settle with the outfitting screen or a measurement |
| 0x1C, 0x20, 0x24 | forward, reverse, lateral/vertical acceleration (m/s^2) | forward 44.85 at 4 ENG pips, 43.42 at 0; reverse 30.26; lateral 30.23 | D: session 2's speed runs fitted (`data::measured`, the traces in `crates/elite-sim/tests/traces/`); EDSY's 44.39, 29.96 were the hints |
| 0x44..0x4C | angular accelerations (deg/s^2) | pitch 220, yaw 110, roll 240 | **provisional**: EDSY's `pitchacc`, `yawacc`, `rollacc` (E8), until measured |
| 0x2C, 0x5C..0x64 | the flight assist's linear and angular time constants (D, R) | D 0.990; R not known | D measured (session 2's fit, `data::measured::FA_D`); R a stand-in (0: the code's own floor) until measured |
| curves 0x68.., 0x78.., 0x88.. | the blue zone's (a, b, p) per curve | pitch/yaw a 0.4456; the rest not known | a measured (D: session 2, pitch at rest, `data::measured::TURN_CURVE_A`); b, p and roll's curve stand-ins until session 3 |
| boost fields 0..7 | warm-up, firing, cool-down times, energy, drain | cost 7 MJ, interval 4 s (B, coriolis); the rest not known | B / D |

Nothing here is tuned by feel: a value is the game's (B, C), a measurement (D), or a hint marked provisional until measured.

## Rules (step 0.10, the first study session)

"Block" is the performance block above. `s` the ship's speed, `V` the speed cap, `dt` the frame's time.

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| Speed cap | V = top speed x g_lin (g_lin 1 normally); while boosting V = boost speed | none in exe (top speed, boost speed are block 0x00, 0x0C) | C | 0x142E6E960 |
| Target velocity (FA on) | v* = stick (lateral, vertical, forward) x V; if a minimum forward speed c is set, forward = c + (V - c) x stick; scale to V if longer; clamp lateral/vertical to +- block 0x08 x V, backward to - block 0x04 x V, forward to V | - | C | 0x142E6E960 |
| FA time constant (linear) | T = max(0.5, D x max(0, 4/3 x s / Vfull - 1/3)); tau = max(T, dt); D = block 0x2C (boost: boost field 5) | 0.5, 4/3, 1/3 | C | 0x142E6E960 |
| Thrust (FA on) | a = (v* - v) / tau - outside pull, clamped per axis to the acceleration limits, jerk-limited, then speed guard; force = a x mass | - | C | 0x142E6E960 |
| Acceleration limits | forward + block 0x1C, reverse - block 0x20, lateral and vertical +- block 0x24, all x g_lin; boosting: forward x boost field 4, x and y also x block 0x30 / 0x34; reverse x 0.25 (no g_lin) when flag +0x5E2 and forward stick >= 0.999; near planets blended toward ship-record values (not worked out) | 0.25, 0.999 | C | 0x142E6AF90 |
| Linear jerk limit | if block 0x28 > 0 and not boosting: |a| <= |a last frame| + block 0x28 x dt | - | C | 0x142E6E960 |
| Speed guard (FA on) | if |v + a dt| > max(s, V), shorten a so |v + a dt| = max(s, V) | - | C | 0x142E6E960 |
| Thrust (FA off) | a_axis = |stick| x (upper limit if stick >= 0 else lower limit), clamped to +- |sign(stick) V - v_axis| / tau; none on an axis already at its cap share moving the stick's way; force = a x mass x (1 - f) | - | C | 0x142E6E960 |
| Over-speed drag (FA off) | when s > full-power top speed: a += 0.25 x reverse acceleration along -v | 0.25 | C | 0x142E6E960 |
| Speed guard (FA off) | limit L = max(s, V) + u x (500 - max(s, V)), u = max(0, cos(angle of new velocity to the outside pull)) | 500 m/s | C | 0x142E6E960 |
| FA on/off switch | f = 0 unless "assist active"; active = supercruise, or (input FA setting != 0 and no override), or a timer running; f = 1 (or 1 - x from an attached helper) | - | C (the mapping to the FlightAssistFinal binding is inferred) | 0x142E4A820, 0x142E6E960 |
| Counter-drift force | extra force against sideways drift, strength (block 0x14 x ...)^3 x 10, growth limited by block 0x18 | 0.5 (data), 10 | C (partial) | 0x142E6E960 |
| Turn rates | W_axis = block 0x38 (x) / 0x40 (y) / 0x3C (z) in deg/s x pi/180 x g_rot (the speed factor applies to the clamp's bound only: see the next rows); pitch/yaw/roll naming of the axes inferred | pi/180 | C | 0x142E6E960 |
| Angular acceleration limit | A_axis = block 0x44 / 0x48 / 0x4C in deg/s^2 x pi/180 x g_rot x curve 0x68.. | - | C | 0x142E6E960 |
| Rotation (FA on) | alpha = (clamp(stick x W_raw + corrections, +-F x W_raw) - omega) / tau_r (corrected 2026-11-14, `rotation-clamp.md`, from the disassembly: the stick scales the raw rate W_raw = block 0x38/0x40/0x3C x pi/180 x g_rot, and only the clamp's bound carries the speed curve F: the turn curve 0x78.. for x, y, the roll curve 0x88.. for z; the acceleration curve 0x68.. bounds only A; boosting, the curves become f = 0.1 t + block 0x84 and W_raw is also scaled by f), tau_r = max(dt, R x max(0, 10/9 x s / Vtop - 1/9)), R = block 0x5C..0x64; clamp +-A; angular jerk limit block 0x50..0x58 (deg/s^3) | 10/9, 1/9 | C | 0x142E6E960 |
| Rotation (FA off) | alpha = A x stick per axis; none on an axis already at W turning the stick's way | - | C | 0x142E6E960 |
| Rotation reversal modes | block 0xA4 = 1: alpha x (1 + block 0xA8..) where |stick| > 0.2 opposes the spin; = 2: spin scaled by block 0xB4.. there | 0.2 | C | 0x142E6E960 |
| Blue zone (speed curve) | x = clamp(s / V, 0, 1); x <= p: a + (1 - a) x / p; else b + (1 - b)(1 - (x - p)/(1 - p)); (a, b, p) per curve from the block | - | C | 0x142E50650 |
| Turning while/after boost | within 3 t s after a boost (and during it): turn factor 0.1 t + block 0x84, z factor block 0x94, accel factor block 0x74 | 3.0, 0.1 (data) | C | 0x142E6E960 |
| Thruster mass curve | x = clamp((m - mmin)/(mmax - mmin), 0, 1); e = log((kopt - kmin)/(kmax - kmin)) / log(1 - (mopt - mmin)/(mmax - mmin)) (e = 1 if that share >= 1); k = kmin + (kmax - kmin)(1 - x)^e | none in exe (curves are module data) | C | 0x142CAC400 |
| Mass curve use | 9 curves in the thruster data: speeds (0x00, 0x0C), accelerations (0x1C..0x24), turn rates (0x38..0x40), the three boost curve values, boost accel multipliers, and one added to the boost ramp time | - | C | 0x142EBDF60, 0x141DFC6E0 |
| Pip blend (ENG) | current block = base + (full-power - base) x t field by field (negative full-power values skipped); t = ENG share (inferred); full-power block uses t = 1 | - | C (t's meaning inferred) | 0x142E86E20, 0x141B2E310 |
| Speed multipliers | top and boost speed x k, k = min(A, B) x C x module factor x FSD-object factor; boost speed also x module factor and x boost field 7 | - | C (A, B, C unnamed) | 0x142E86E20 |
| Boost | states idle / warm-up (field 0 s) / firing (field 1 s) / cool-down (field 2 s); energy cost field 3 (taken once at the press); field 6 is heat per second while firing (corrected by A20; it was read as a capacitor drain); stick pushed to full forward over block 0x10 s | - | C | 0x142E4A820 |
| Torque | torque = s x M x alpha; alpha = the angular acceleration at +0x640 (ship axes, rad/s^2); M = a padded 3x3 matrix from the body (inertia tensor: inferred only); s = a scalar from +0x610, identity unresolved | - | C (form only; s, M and the assist-off path open) | 0x142E6E960 |
| Frame order | each frame: the app fires its update phases in order with the frame's dt; the physics world steps (phase 0x145F05B20) before the flight step (phase 0x145F05B2C), so a frame's thrust is integrated by the next frame's sub-steps (inferred from the order) | - | C (order); integration link inferred | 0x1401EB050, 0x1422709F0, 0x142E6E960 |
| Flight step dt | the flight step uses the frame's dt (variable), not the physics sub-step | - | C | 0x1401EB050, 0x142E22500 |
| Physics sub-steps (in play) | n = clamp(floor((leftover + frame dt) / h), 1, N) equal sub-steps of (leftover + frame dt) / n; leftover = 0 (always chosen while the byte at 0x145E83EC4 is nonzero: it is 1 in the exe, no writer found) | h = 1 / f; f, N not found | C (code); "in play" inferred | 0x1422709F0, 0x14066E690 |
| Physics sub-steps (other branch) | when that byte is 0: n = min(floor(acc / h), N) sub-steps of exactly h, leftover = fmod(acc, h), except when acc >= (N + 1) h, which takes the even split above | - | C | 0x1422709F0, 0x14066E0F0 |
| Physics step size | h = 1 / f, f = the PhysicsWorld component's parameter +0x10 (Hz); replaced by another service's rate when that is active and not a multiple of f (fmodf >= 0.001) | 1.0, 0.001 | C (the services unnamed) | 0x14228D5E0 |

## Rules (step 0.11, the second study session)

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| Mouse settings (defaults, ranges) | MouseSensitivity 0.1..10 (default 1); MouseDeadzone 0..0.2 (0.05); MouseLinearity 1..3 (1); MouseDecayRate 1..10 (4); MouseXDecay/YDecay bools (exe default 1; KeyboardMouseOnly.binds sets 0) | table 0x145E77BD0 | C (table); that the ship's mouse node reads these by name is inferred from their roles (the node reads its setting names from its descriptor) | 0x1408FE740, 0x145E77BD0 |
| Mouse to stick: accumulate | each frame d = mouse delta x MouseSensitivity / R; descriptor mode 0: v = clamp(v + d, -1, 1) (virtual joystick); other modes: v = d (this frame's movement only) | -1, 1 | C (rule); R and which mode the ship's pitch/roll use: not found (descriptor fields) | 0x1408F2210 |
| Mouse decay | when the axis's decay setting is on: v = v x 0.5^(MouseDecayRate x dt) before the new delta is added (halves every 1/rate s) | 0.5 | C | 0x1408F2210 |
| Mouse reset | the reset input sets v = 0; v is also 0 (no output) with no mode set, in UI focus, or in a ship state 3 | - | C | 0x1408F2210 |
| Mouse dead zone and curve | z = MouseDeadzone, e = MouseLinearity: x = (abs(v) - z)/(1 - z) if abs(v) > z else 0; stick = sign(v) x x^e; written to the pitch / roll / yaw value named by MouseXMode/MouseYMode (Roll and PitchInverted flip the sign: mouse right gives negative roll), only if larger in magnitude than that value's other sources this frame | - | C | 0x1408F2210 |
| Mouse full deflection R | the mouse counts that move v by 1 (divided by sensitivity) | - | not found (descriptor +0x98; descriptor source not traced, presumably data) | - |
| Pip share t (settles "Pip blend" t) | t = clamp(pips_ENG / max pips per channel, 0, 1); channel 1 = engines (boost energy is drawn from channel 1) | max pips: data (parameter byte +0x13) | C (formula; that +0x930 is the distributor is inferred by matching its two calls); the numbers not found (data); t = ENG pips / 4 if the UI's 4-pip maximum holds (inferred) | 0x1437FEF60, 0x142E86E20 |
| Pip moves | one press moves min(max - pips, step) pips into the channel, first from unassigned pips, then from the others | starting pips, max, total, step: data (parameter bytes +0x10..+0x16) | C (rule); numbers not found | 0x1437FF4E0, 0x1437FADA0 |
| Capacitor recharge | per channel c each frame: rate_c x dt with rate_c = lo_c + (hi_c - lo_c) x s_c^p_c, s_c = clamp(pips_c / max pips, 0, 1); 0 while the power plant gives nothing; added to the charge up to capacity (inferred) | lo, hi, p: distributor module data (record +0x04, +0x08, +0x0C per channel) | C (expression); its use inferred; numbers from module data | 0x1437FD920, 0x143800240 |
| Speed multipliers A, B, C (start values) | the three doubles start at 1.0 in the constructor; writer not found (bounded search; leads 0x142E825A0, 0x142E37C10 unresolved) | 1.0 | C (start value); later changes open | 0x142E32DD0, 0x142E86E20 |
| Minimum forward speed c (+0x688) (start value) | starts at 0 in the constructor; writer not found (bounded search; lead 0x142E825A0 unresolved); link to Coriolis's minthrust not settled | 0 | C (start value); later changes open | 0x142E32DD0, 0x142E6E960 |
| Simulation rate (Hz) and N | the PhysicsWorld component's parameter object (+0x10, +0x14); presumably data like the distributor's parameters (inferred) | - | not found (D) | 0x14228D5E0 |
| Torque (settles A10's s) | torque = f x M x alpha_assist + (1 - f) x M x alpha_raw, two calls of body +0x50; f = the assist weight (+0x610) | - | C | 0x142E6E960 |
| Rotation damping (FA off) | on each axis whose stick is centred (abs(stick) <= 0.001), while another axis's stick is deflected and speed <= V + 0.1: alpha_raw += K_axis x (-omega_axis), K = block 0xC0 / 0xC4 / 0xC8 (x, y, z) | 0.001, 0.1 | C (rule; K's meaning inferred, numbers ship data) | 0x142E6E960 |
| Mass lock | - | - | not studied (no time) | - |

**G2 (step 0.11):** every flight rule is now C (found, with its address) or D (to be fitted from step 0.9's measured traces):
the simulation's rate and sub-step cap, the mouse's counts for full deflection, the pips' counts, the capacitors' recharge
numbers, and mass lock (not studied) are D. Glide and supercruise are steps 5.1 and 6.1's own sessions.

## Rules (A20, the ship's systems a: distributor, boost energy, heat, fuel; for steps 2.1, 2.2, 2.5)

Notes: `re/elite/ghidra/notes/2.x-systems-a.md` (out of git). Two earlier rows corrected: the boost's field 6 is heat per
second while it fires, not a capacitor drain (the boost's only energy cost is field 3, taken at the press); the pip
parameter byte +0x14 is the unassigned pool at a reset, not the total pips.

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| Distributor channels | channel 0 = WEP, 1 = ENG, 2 = SYS; per channel record: +0x00 capacity (outfitting "Capacity"), +0x08 = hi (outfitting "Recharge": the rate with all pips there), +0x04 lo and +0x0C p shown nowhere | - | C (mapping, from the engineering modifiers' field ids); lo, p not found (D) | 0x142135360, 0x142135530, 0x141B3A480, 0x141B386D0 |
| Draw energy (all or nothing) | if charge < E: refused (with flag F the channel is emptied, else untouched); else charge -= E | - | C | 0x143801BC0 |
| Draw energy (what there is) | charge -= min(charge, E) | - | C | 0x1438021C0 |
| Pip reset | pips per channel = starting bytes (parameter +0x10..+0x12), unassigned pool = parameter byte +0x14 | numbers: data | C (rule; that it is the reset key inferred); 2/2/2 not confirmed (D) | 0x143801FA0 |
| Boost energy check | at the press: engine off -> `$INFO_boost_denied_engine_off;`; else draw boost field 3 from ENG all or nothing (flag 0); refused -> `$INFO_boost_denied_not_enough_energy;`, capacitor untouched | cost: data (Coriolis 7 MJ) | C | 0x142E4A820 |
| Boost heat (corrects A10's "field 6 capacitor drain") | while firing: heat += boost field 6 x dt (label "Engines"); no energy drain while firing | field 6: data | C | 0x142E4A820, 0x143813740 |
| Planetary-flight ENG drain | when planetary flight is on: E = max over axes of weighted thrust share (<= 1) x L x G; draws E x record +0x2A8 x dt from ENG; refusal raises a 0..1 cut (rate 1/s) that lowers the acceleration limits | records: data; L, G not identified | C (form); "planetary" inferred from its heat label | 0x142E6E960 |
| Planetary-flight heat | heat += record +0x2F4 x sum over axes of max(two share-weighted terms with lerp(0.5, 1, share)) x dt | 0.5 | C (form) | 0x142E6E960 |
| Heat capacity, start | capacity = ship record +0xF0; heat starts at 0.4 x capacity | 0.4 | C | 0x143813D20 |
| Add heat | Q = clamp(c x capacity - heat, 0, Q) when a cap c is given; heat = max(0, heat + Q) | - | C | 0x143813740 |
| Heat tick | docked: heat += parameter +0x14 x dt while heat < parameter +0x18; then heat = f_n(..f_1(heat, k, dt)) through the registered heat objects (list B); k starts at 1 | - | C (structure); one f_i is the heat radiator's dissipation (row below), any others not found | 0x14380E370, 0x1438144A0 |
| Heat proportion | `HeatProp` = heat / capacity (`HeatPropClamped` clamped to 0..1) | - | C | 0x14380E370, 0x1438139E0 |
| Heat warning | voice `$HeatWarning100;` + journal `HeatWarning` when heat/capacity rises through 1.5, at most every 5 s | 1.5, 5.0 | C (rule); the HUD % is settled: see "The heat scale" at the end | 0x142DC1940 |
| Heat damage | above t0 = record +0xF4 (heat/capacity): rate = lerp(+0xFC, max(+0xFC, +0x100), clamp((f - t0)/(+0xF8 - t0), 0, 1)), accumulated and dealt every record +0x104 s; voice `$HeatWarningDamage;` + journal `HeatDamage` when f rises through t0, at most every 5 s | 5.0; the rest ship data | C (rule); numbers D | 0x142DC1940 |
| Heat dissipation | each tick heat = max(0, heat - rate x dt); rate = k x (R+0x04 + (R+0x08 - R+0x04) x s^2), s = clamp((f - R+0x18)/(R+0x1C - R+0x18), 0, 1), f = heat/capacity, s = 0 when unpowered; rate 0 under two switches (silent running likely, inferred) | 2.0 | C (rule); being the heat manager's processor inferred (registration not seen); R = the heat radiator's data (R+0x04, +0x08 = EDSY heatdismin 1.18, heatdismax 18.15 by name, inferred; +0x18, +0x1C D) | 0x144398F40, 0x144399D90 |
| FSD charging heat | while charging: heat += (rec+0x0C + rec+0x10 x max(0, t - rec+0x04)) x dt, t = charge time; later state: rec+0x0C x dt | rec: drive data | C (form) | 0x142C71440 |
| Star heat | per star: x = distance / radius - 1; heat += P+0x18 x clamp((x - P+0x20)/(P+0x1C - P+0x20), 0, 1) (x P+0x24 unless a test holds) x dt | P: data | C (form) | 0x142C83F60 |
| Silent running's switch (found 2026-11-16) | silent running is the radiator's own switch: the silent-running button (`ToggleButtonUpInput`) shows byte +0xA5, the same byte that zeroes the radiator's rate. That it also drops the shields is the game's known behaviour (B), not found in the exe | - | C (the radiator); B (the shields) | 0x144399D90, 0x142D0CD20 |
| Power plant heat, thruster heat (normal flight), heat sinks | - | - | not found (D) | - |
| Fuel burn | consumers each frame return their burn; all of it comes from the reservoir: tonnes = burn / (+0x2D0 x 45000) | 45000.0 | C (form); +0x2D0 not found | 0x14380AE40 |
| Reservoir refill | when the reservoir is empty: one reservoir capacity moves from main (or all of main if less); journal `ReservoirReplenished` | - | C | 0x14380C6E0 |
| Fuel available, unlimited fuel | available = (main + reservoir levels) x +0x2D0 x 45000; an obfuscated flag (+0x228) makes it unlimited (no burn) | 45000.0 | C | 0x14380B790, 0x14380AE40 |
| Low-fuel warnings | fuel / (main + reservoir capacity) crossing down through 0.25, 0.10, 0.05 in a frame: `$WARNING_Fuel_25;` / `_10;` / `_05;` | 0.25, 0.10, 0.05 | C | 0x14380D3C0 |
| Power plant fuel | need = dt x a x b (a, b two values of the plant's output state: rate and load share inferred); short of fuel: output x (fuel left / need), voice `$GeneratorNoFuel;`; plant off: no burn | - | C (form); a, b's meaning inferred, numbers data | 0x1438011E0 |
| Power plant load | load = clamp(sum of consumers' draw / plant capacity, 0, 1) | 1.19e-7 | C | 0x1437FCEE0 |

## Rules (A21, the ship's systems b: shields, hull, landing, lasers; for steps 2.3, 2.4, 2.6)

Notes: `re/elite/ghidra/notes/2.x-systems-b.md` (out of git). "Gen record" = the shield generator's module data.

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| Shield mass curve | k = k_lo + (k_hi - k_lo)(1 - x)^e, x = clamp((m - mmin)/(mmax - mmin), 0, 1), e as the thrusters' curve; m <= mmin gives k_hi, m >= mmax gives k_lo; gen record +0x5C..+0x64 masses, +0x68 k at max mass, +0x6C k at optimal, +0x70 k at min mass | 1.0 if opt mass <= 0 | C | 0x142CAC400, 0x143879360, 0x14387EB40 |
| Shield strength | live module: k(m) x ship base shield (ship record +0x0C), m from a referenced component (not identified); ship totals: k(ship record +0x00, hull mass inferred) x base shield x A + B with one generator fitted, else 0; A, B the shield multiplier/addition (names inferred) | - | C (form); m and A, B sums not traced | 0x14387EB40, 0x141DFC630, 0x141DFC570 |
| Hull strength | hull = ship record +0x08 (base armour, inferred) x C + D; C, D the health multiplier/addition (names inferred) | - | C (form) | 0x141DFC630, 0x141DFC570 |
| Shield regen | only when flag +0x687 is set (writer not found) or the shield is up; then when powered, below max and time since last hit >= delay: S += X x dt x rate x F, capped at max; rate = lerp(gen +0x30, +0x34, s) (up) or lerp(+0x38, +0x3C, s) (broken); s = distributor channel 2 share (SYS, inferred from the field types' WEP, ENG, SYS order), 0.5 without a distributor; F = 20 / 20 / 2 in three states not identified, else 1; X a multiplier not identified | 20, 2, 0.5 | C | 0x14387A960 |
| Shield regen delay | up: gen +0x4C s; broken: lerp(gen +0x50, +0x54, s), 16 s without a record; any hit with damage > 0 restarts the timer | 16 | C | 0x14387A960, 0x14387E850, 0x143877C10 |
| Shield regen energy and heat | energy drawn = (gain / F) x lerp(gen +0x44, +0x48, s), the gain applied only if the draw succeeds; heat = (gain / F) x gen +0x40 | - | C (heat role inferred) | 0x14387A960 |
| Shield collapse and return | collapsed when S <= FLT_EPSILON or unpowered (unpowered: S = 0); back up when S > max x gen +0x58; on power-on with S <= 0, set to max x 0.5 (inferred) | 1.19e-7, 0.5 (.data) | C | 0x14387A960 |
| Shield hit | mitigated damage T (7 damage types, resistances applied elsewhere); if T >= S: S = 0 and the hit scaled by (T - S)/T goes on (to the hull, inferred); else S -= T (that store not followed) | - | C (partial) | 0x143877C10 |
| Hit penetration scale | unless hit flag 0x200: r = min(hit +0x2C / Y, 1), Y from the ship-side object; hit scaled by r when r < 1 (penetration / hardness: inferred) | 1 | C (names inferred) | 0x143877C10 (0x14387817B) |
| Shield resistances | - | - | not found (data) | - |
| Life support (found 2026-11-16) | two clocks, one for a breached canopy and one for the module without power (each 0 while its cause is absent). Past a grace period (module data +0x18 / +0x14) the emergency oxygen drops 1 s a second; at 0, with a breach or no power, the commander dies. Otherwise the oxygen is refilled (the exe writes `max(full, o + rate x dt)`; the mod holds it at full) | 300 s for the 1E module (B) | C; the grace periods are data (0 used, E) | 0x142EC3180, 0x142ECF1C0, 0x142EC80C0 |
| Canopy breached (found 2026-11-16) | the canopy counts as breached while its own health is at or under 0. What takes its health down was not found: the mod breaks it on a hit that leaves under a tenth of the hull (E) | - | C (the test); E (the cause) | 0x1430C6130 |
| Life support alarm (found 2026-11-16) | critical at 30 s of oxygen or less | 30.0 (0x144DE168C) | C | 0x142D59590 |
| Collision damage, hull 0 | - | - | not studied (leads in the notes) | - |
| Touchdown allowed | 0 <= h <= P14, speed <= P10 (velocity inferred), angle between ship down and gravity <= theta (landing record +0x38 deg, 60 deg without one); P10, P14 the landing component's parameters (data) | 60 deg | C (the +0x200 state gate not identified) | 0x1430ECE00, 0x1430F2FA0 |
| Landing approach and footing | approach flag at h <= P1C; 4 rays along gravity (h + 100 m) from the landing points; all 4 must agree within 0.5 m to go on | 100, 0.5 (.data) | C (counting loop's reading inferred) | 0x1430ECE00 |
| "Weapon Heat Dump" call | once a frame while a list is non-empty: a heat-type object gets minus record +0x148 (x dt if record +0x130 set); role not identified (heat removal or thermal load) | - | C (code); role open | 0x142F38830 |
| Fire interval, WEP draw, damage falloff, deploy, gimbals, landing gear, liftoff | - | - | not found / not studied | - |

## Rules (A50, the frame shift drive; for steps 5.1-5.4)

Notes: `re/elite/ghidra/notes/5.1-fsd.md` (out of git). P = the ship record's supercruise floats at +0x1BC (P[k] = byte offset k), Vsc = the supercruise block's top speed (ship record +0xF0), D = the FSD module's data, Fp = the FSD component's parameters, K = the cruise component's parameters: all data (D).

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| Well slowdown, stars (body kind 1) | d' = d - 1.1 R; x = clamp(d'/(12 AU), 0, 1); s = min(max((1 - x)^2, 1 - log(d/AU x P[0x04] + P[0x00]) / log(10000 P[0x04] + P[0x00])), 1 - P[0x0C]/Vsc) | 1.1, 12 AU, 10000, AU = 1.49598e11 m | C (R = radius inferred) | 0x142E96C30 |
| Well slowdown, kinds 2 and 7 | M = a value stored on the body (mass inferred, unit not resolved); W = (M^0.1 + 1) x 20 AU; x = clamp(d'/W, 0, 1); s = min((1 - x)^2, 1 - P[0x08]/Vsc) | 0.1, 20 (.data), 1.1 | C (form); M's unit open | 0x142E96C30 |
| Well slowdown, combined | s = max over the system's bodies (and the nav target if a virtual object or FSD cloud, a variant with closing speed) | - | C | 0x142EA13E0, 0x142E96A30 |
| Supercruise speed multiplier (settles A11's +0x670, +0x688 in supercruise) | m0 = 1 - (s + (1 - max(P[0x40], F)/Vsc - s) w), F = P[0x3C]; with a target in mode 1, m = m0 (1 + (P[0x4C] - 1)(1 - clamp((theta - P[0x44])/(P[0x48] - P[0x44]), 0, 1))), theta = |arcsin(a . u)| in degrees, u = direction to the target, a = a ship axis (which: open); controller +0x670 = m, accel mode +0x68C = 2, min forward speed +0x688 = P[0x10] + (F - P[0x10]) w | - | C (w's source, cruise +0x3B0, unidentified) | 0x142EA9810, 0x142E79C60, 0x142E780C0 |
| Supercruise top speed | V = Vsc x m x min(A, B) x thruster and FSD factors = Vsc (1 - s) plainly; floors P[0x08] (planets), P[0x0C] (stars); throttle and accelerations by the assist-on rule with the supercruise block | - | C | 0x142E86E20, 0x142EA9810 |
| Accel mode 2 (supercruise) | top speed, forward and lateral accel x k; reverse accel = max(r k, r (v/V0) max(1, sqrt(v/(V0 k)))), v = ship speed (+0x6A0, inferred), V0 = block top speed | 1 | C | 0x141B2D8F0 |
| Safe drop thresholds | x = clamp((Vt/c - 1)/(Vsc/c - 1), 0, 1)^0.25, Vt = (1 - s at the target) Vsc; Dmin = P[0x18] + (10 P[0x28] - P[0x18]) x; Dmax = P[0x1C] + (10 P[0x2C] - P[0x1C]) x; Vdrop = P[0x20] + (10 P[0x30] - P[0x20]) x; moving target: Dmax += 2 v_t, Dmin scaled alike, Vdrop += 0.1 v_t | 0.25, 10, 2.0, 0.1 | C (partial reading of scrambled values) | 0x142E9A080 |
| Safe drop conditions | drop at the target (exit kind 1) when Dmin < distance < Dmax (a window; checked on the disassembly), relative speed < Vdrop, angle between the ship's +z (nose, inferred) and the target < P[0x24] deg | pi/180 | C (centre vs surface distance open; that it waits for the FSD key is a lead: a status byte +0xC0 gates it) | 0x142E9A080, 0x142EA1F20 |
| Emergency drop effects | kinds 2, 3: u = clamp((v/c)/K[0x18], 0, 1), k = K[0x1C] + (1 - K[0x1C]) u; kick K[0x10] k, spin K[0x14] k; 100 with label "EmergencyDrop" to an unidentified receiver in the heat-add call shape (heat += 100 inferred); random module damage; f x K[0x38] (f 0.1 kind 2, 1 kind 3) to an object at +0x3C8 (role open) | 100, 0.1 | C (effects; the heat inferred); triggers not found (D; lead: 0x142EA3DA0 sets the exit kind at +0x530) | 0x142EA1F20 |
| Drop and arrival cooldowns | after leaving supercruise: Arriving 5 s, then cooldown Fp[0x14] (or Fp[0x18] when a +0x1B0 query says so); after a hyperspace jump: Arriving 1 s, then Fp[0x18]; requests during the cooldown get `denied_cooldown` | 5.0, 1.0 | C | 0x142CB6D80, 0x142CB6FD0, 0x142C71440 |
| FSD refusals (order) | disabled, malfunction, cooldown, disrupted, mass lock, hardpoints, landing gear, cargo scoop, deployables, clear path, fuel too great (> max fuel), insufficient fuel, permits, overcharge | - | C | 0x142C71440 |
| Charge time and rate | hyperspace: time D[0x04], rate 1/s; supercruise: time Fp[0x1C], rate 1/L (L = mass-lock factor from +0x5D8, re-read every 0.5 s charging, 3 s otherwise; locked -> rate 0 -> `denied_mass`); x5 under an unidentified override; HUD factor round(L) when > 1 | 0.5 s, 3.0 s, 5.0 | C (L's own rule not found) | 0x142C71440, 0x142CAA540 |
| Jump conditions at charge end | first failing gives the HUD code: 1 `requires_alignment` (nose to destination <= Fp[0x30] deg), 2 `requires_alignment_escape` (supercruise entry: nose against a direction from the nearby body <= Fp[0x30], Fp[0x34] in one planet case), 3 `requires_alignment_velocity` (nose to velocity <= Fp[0x30]), 4 `requires_speed` (rule not read: FSD interface virtual +0x180) | - | C (codes 1-3); code 4's rule D | 0x142CC61F0, 0x142CC6420, 0x142CC66E0, 0x142C71440, 0x142D6DEE0 |
| Countdown | Departing: a ticker 0 -> 1 at dt/5 (about 5 s), heat D[0x0C] per s; a failed condition during charge returns to ready | 5.0 | C (what ends it at 1: inferred) | 0x142C71440 |
| Jump fuel use | jump fuel computed at charge start (FSD interface virtual +0x140), taken at departure as min(aboard, it); max per jump = D[0x24] x b_f (booster factor, or 1) | - | C | 0x142C71440, 0x142C9B000 |
| Jump fuel formula (settles fsd.rs's form) | f' = min(fuel aboard, D[0x24]); q = m/D[0x14]; R0 = (f'/D[0x18])^(1/D[0x1C]) / q; R1 = (R0 + b_a) k_r; if R0, R1 > 0 and differ, d = d R0/R1; fuel = D[0x18] (q d)^D[0x1C] b_f (b_a booster added range, b_f booster factor, k_r a range multiplier, 1 normally: inferred) | - | C (D[0x14] opt mass, D[0x18] multiplier, D[0x1C] power by their places) | 0x141B39210, 0x142C9A960 |
| Jump range formula | R = ((min(f, D[0x24])/D[0x18])^(1/D[0x1C]) / (m/D[0x14]) + b_a) k_r; m = total mass with the asked cargo and fuel | - | C (fsd.rs's jump_range when b_a = 0, k_r = 1) | 0x141B392E0, 0x142C9ABD0 |
| Star heat | per star: f = d/R - 1; rate = clamp((f - Q[0x20])/(Q[0x1C] - Q[0x20]), 0, 1) x Q[0x18] (x Q[0x24] unless a query says not); heat += rate x dt ("Star Proximity"); a flag when f < Q[0x20] (meaning depends on the data's ordering) | - | C (form); Q data (D) | 0x142C83F60 |
| Exclusion zone registration | a CruiseExclusionZone registers its owner with a parameter float (a radius, inferred) x 1.05; its use not traced | 1.05 | C (registration only) | 0x142E06850 |
| Hyperspace arrival position, mass-lock factor's rule, emergency-drop triggers, the requires_speed rule, overcharge | - | - | not found (D) | - |

## Rules (A60, planetary approach: orbital cruise and glide; for steps 6.1, 6.2)

Notes: `re/elite/ghidra/notes/6.1-planets.md` (out of git). Rows from A60 (planetary approach: glide and orbital cruise, spec step 6.1's study session, 2026-11-14). Details: `re/elite/ghidra/notes/6.1-planets.md`. No scrambled constant was read. G = the ship component with the glide state machine (the unit that also holds the CruiseArrivalState / CruiseArrivalParams network classes), S = the ship / navigation-target object, theta = the glide pitch in degrees. "CAP" = the `CruiseArrivalParams` struct at G+0x5B0 (names are the exe's own): maxDecelerationDistance D, endSpeed, speedLimit, maxDecelerationDuration T, power p, steepestDivePitch, shallowestDivePitch, alignmentGracePeriod: **the numbers are data (D), not in the exe**.

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| Glide pitch | theta = asin(-(d . n)) in degrees, d = unit vector from the ship to the target body's centre, n = the ship transform's row +0x20 (the nose, inferred): nose at the centre -90, level 0, climbing positive | 57.29578 | C (n = nose inferred) | 0x142C78160 |
| Glide band | steep if theta <= steepestDivePitch; else shallow if theta >= shallowestDivePitch; else in band; gauge g = (theta - mid)/(shallowest - mid), mid = (steepest + shallowest)/2 | 0.5 | C (limits D) | 0x142C761B0 |
| Glide preconditions | S mode == 2 and a target body, else end reason 1 NoPlanet; speedLimit not NaN and < 40,000, else end reason 0 SetupIncorrect (speedLimit starts FLT_MAX) | 40,000 | C | 0x142C761B0 |
| Glide entry (first frame) | in band: voice ApproachGlide_Engage, glide goes on; shallow: end reason 6 TooShallowOnEntry; steep: heat add ship record +0x320 ("Planet Arrival") and end reason 4 TooSteepOnEntry | - | C (heat number D) | 0x142C761B0 |
| Glide grace and failure | out of band: timer += dt; back in band: timer = 0; timer >= alignmentGracePeriod: shallow -> SafeAbort, end reason 7 TooShallow; steep -> DangerFail, heat add ship record +0x320, end reason 5 TooSteep; while steep and under the grace: heat += ship record +0x31C x dt ("Planet Arrival") | - | C (grace, heats D) | 0x142C761B0 |
| Glide motion | each frame the body's velocity is set to speedLimit x the ship's nose axis (constant speed, steered by the nose); a demand vector at G+0x4E0 is still added to the body's force accumulator (not decoded) | speedLimit D | C | 0x142CAF260 |
| Glide end tests | ray from the ship along the nose (1,000,000 m long) hits the body nearer than 10,000 m: end reason 3 TerrainDanger; ray from the ship to the body's centre hits nearer than 6,500 m: end reason 2 AltitudeTooLow (the normal end); both play ApproachGlide_Complete; the returned hit value read as metres (inferred from the constants) | 1e6, 10,000, 6,500 | C (units of the hit value inferred) | 0x142CBA8D0 |
| Glide end reasons | 0 SetupIncorrect, 1 NoPlanet, 2 AltitudeTooLow, 3 TerrainDanger, 4 TooSteepOnEntry, 5 TooSteep, 6 TooShallowOnEntry, 7 TooShallow (journal text glideEndedReason=); every reason, good or bad, starts the same deceleration; no damage or drop is in this code | - | C | 0x142CB68E0, 0x142CAF260 |
| Glide deceleration | t = min(t + dt, T); e(t) = (1 - t/T)^p; step = min(speedLimit x dt, D x (e(t_old) - e(t_new))); floor v_f = max(endSpeed, a value from an object at G+0x270); if D (e_old - e_new) < v_f dt the step is v_f dt and it is over (the only way it ends: at t = T the step is 0, under the floor); velocity = step/dt x nose; over at t = T or at the floor (state 2; journal CruiseArrivalDecelerationComplete with duration and distance); speed D p/T (1 - t/T)^(p-1) capped by speedLimit | D, T, p, endSpeed D (T = 1.5 s, p = 4.0 are only the struct's defaults: the sender overwrites) | C (form) | 0x142C75DE0, 0x142C5F400 |
| Glide state byte | 3 idle (constructor), 0 gliding, 1 decelerating, 2 finished; the store of 0 not found; the engaged flag +0x628 is cleared when the struct is filled (0x142CA4BD0) | - | C (1, 2, 3); 0 inferred | 0x142CAF260, 0x142C75DE0, 0x142C5F400 |
| Orbital cruise safe stop (the drop to glide) | in supercruise a swept test of size ship record +0x1D0 (a distance, data) runs from last frame's place to this frame's against the bodies; on a hit with a planet of access class 0: v = -(S virtual +0x68) (read as a closing speed, inferred), z = ship record +0x1F0, f = ship record +0x1F4 (P[0x34], P[0x38] of A50: speed thresholds, inferred); no target body or v >= z + 1,000: emergency stop (TooFastNearPlanet, exit kind 3); S mode 0 nothing; mode 1: emergency if v > 2 (z + (f - z) x S virtual +0x38); mode 2: v < f gives OrbitalCruiseSafeStop (exit kind 4) else emergency | 1,000, 2.0 | C (form); z, f, +0x1D0 data; the meaning of S's mode and floats open | 0x142CAE4A0 |
| Drop to glide hand-over (exit kind 4) | the exit pose is moved 3 x s ahead along the ship's +z and the velocity set to s along the nose (s = 2,500 m/s, or the cruise object's float +0x510 when +0x528 is set), with a further 3.0 argument | s = 2,500, 3.0 | C (the 3.0 argument's meaning inferred) | 0x142EA1F20 |
| Plain drop-out deceleration | after a drop the cruise component slows the ship over 1.5 s with (1 - t/1.5)^6, distance scale from the cruise object's virtual +0xD8 | 1.5, 6.0 | C | 0x142CAE4A0 |
| Planet access class | 0 free; 1 not approachable; 3, 4 missing expansion entitlement; 5 missing planetary approach suite (fitted count >= 1, or >= 2 when body field +0x3DC is nonzero: basic vs advanced, inferred); 7 install incomplete; 8 permit needed; classes 5, 7, 8 end a cruise approach with a safe stop (exit kind 1) and a voice line | - | C (form); suite reading inferred | 0x142C9B660, 0x142CAE4A0 |
| Exclusion shell (permit-locked bodies only) | inside radius a + Gp[0x18] (b - a) of a body that needs an entitlement or permit the ship is moved out to the sphere and told TooCloseToPlanet every Gp[0x1C] s (Gp = G's own parameter record at +0x50, not A50's Fp); class 0 bodies have none | Gp data | C (form); a, b inferred | 0x142C78F60, 0x142C793B0 |
| Gravity and size in the code read | no gravity or mass term in the glide check, motion, end tests, deceleration, the safe stop or the hand-over; size enters only through geometry (the horizon of theta, the surface under the ship, the planet within +0x1D0 of the path); the fixed numbers (10,000 m, 6,500 m, 40,000, 2,500 m/s) do not scale with the radius; the unread component that sends CruiseArrivalParams may depend on the body (open) | - | C for the code read | whole of the above |
| CruiseArrivalParams numbers and their sender (lead: the component of type id 0x145F2F260, vt+0x180, called at 0x142CA4BD0), the glide mode flag's setter, the store of state 0, S's mode and floats (orbital cruise's start and surface speed limit beyond A50's well slowdown), the warning voice lines' triggers, ShipMetresToTouchdown | - | - | not found (D) | - |

## The game's own numbers (N1 and N1b-1, 2026-11-16)

Read from the Sidewinder's records in Elite's data (the ship item, its stock modules, its prototype) by the study in
`re/elite/notes/sidewinder_numbers.md` (out of git: every record layout, field and check), and applied in `crates/elite-sim`
by N1b-1. Where a row here corrects one above, this one holds. The measuring plan for what the records lack:
`re/elite/measure/BACKLOG.md`.

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| Where the performance block comes from | the ship item's data object +0x30 is the performance record: base block +0x18, full-power block via +0xE8, supercruise block +0xF0, supercruise floats P +0x1BC, boost record +0x24C; the hull record is data +0x18, the heat record +0x20, the landing record +0x88 | Sidewinder: see `re/elite/notes/sidewinder_numbers.md` | B (data) + C (getters) | 0x141B3AFF0, 0x141B34F80, 0x141B3B2C0, 0x141B3B020, 0x142E512F0 |
| Angular acceleration limit (correction) | A = (block 0x44 for x, 0x4C for y, 0x48 for z) x pi/180 ...; the jerk limits 0x50/0x54/0x58 and R 0x5C/0x60/0x64 are x, y, z in order | pitch 220, yaw 110, roll 240 deg/s^2 | C | 0x142E4A820 (+0x5F0 from +0x794, +0x79C, +0x798) |
| Pip blend (numbers) | the full-power block holds only top speed 220, pitch 43, jerk x 300, R x 0.05, turn curve a 0.4 b 0.3, roll curve b 0.9; everything else -1 (kept from the base: 100 m/s, pitch 35 ...) | t = ENG/4 | B | records |
| minthrust | coriolis's minthrust is the base block's top speed over the full block's (100 / 220); the boost speed has no full value | 45.45 % | B + C | 0x141B2E310 |
| Thruster mass curves | three curves, accelerations (rec +0x0C), speeds (+0x24), turn rates (+0x3C), the same 24/48/72 t, 0.83/1/1.03 for the 2E; the boost curves and the ramp addition are neutral | - | B + C | 0x142EBDF60 |
| Boost (numbers) | warm-up 1 s, firing 2 s, cool-down 1 s; while firing every upper acceleration limit x 4, heat 10 /s; cost 7 MJ | 1, 2, 1, 4, 10, 7 | B | performance record +0x24C |
| Supercruise numbers | Vsc 6e11 m/s (2001.38 c); cap floors 200 km/s (planets), 1e8 m/s (stars); zero-throttle speed 30 km/s; star log constants 1.04, 0.03; drop window in a well under 1,000 km and 1,000 km/s, beyond 20 km (free targets 30,000 km and 30,000 km/s, no minimum); nose within 30 deg | P block | B | record +0x1BC |
| Supercruise accelerations and turns | fwd 4.5e10, rev 9e10, lat 1e11 m/s^2 (x k in mode 2), jerk 1e10; turn pitch 38, yaw 12, roll 110 deg/s with turn curve (0.5, 0.6, 0.5) and roll curve (0.8, 0.8, 0.5); no pip blend | - | B | record +0xF0 |
| Orbital cruise (numbers) | approach speed F = 2,500 m/s (cap and min speed blend to it with w); safe stop: a sphere of 8,000 m along the frame's path meets a free planet, speed under 5,050 m/s gives the glide, over 201,000 m/s an emergency stop | 8000, 5050, 200000, 2500 | B (w's source open) | record +0x1D0, +0x1F0, +0x1F4, P3C |
| Glide heat | too steep: 36 heat /s inside the grace, 72 at the failure | 36, 72 | B | record +0x31C, +0x320 |
| Star heat (numbers) | rate = 18 x clamp((f - 1)/(0.01 - 1), 0, 1) x 0.5 heat /s, f = d/R - 1 | 18, 1, 0.01, 0.5 | B (component link inferred) | ShipBodyProximityEffects template |
| Heat dissipation (numbers) | rate = k (0 + 18.15 s^2), s = clamp(heat/capacity, 0, 1); R+0x00 = 1.18 not read by this code | 0, 18.15, 0, 1 | B (record link inferred) | ext_radiator_standard +0x120 |
| Heat damage (numbers) | from heat/capacity 2.5: 0.5 rising to 1.5 at 3.5, dealt every 2 s | 2.5, 3.5, 0.5, 1.5, 2 | B | heat record +0xF4..+0x104 |
| FSD heat, charge, cooldowns, alignment | charge to supercruise 5 s, jump charge 15 s, heat 10 /s charging and departing, cooldown 5 s after a drop and 40 s after a jump, nose within 10 deg (50 deg escaping a kind 2 body) | 5, 15, 10, 5, 40, 10, 50 | B | Fp, D |
| Touchdown (numbers) | allowed when 0 <= h <= 100 m, speed <= 15 m/s, ship down within 20 deg of gravity; approach flag under 150 m | 15, 100, 150, 20 | B | PlanetLandingModule, landing record +0x38 |
| Life support graces | the emergency oxygen starts 6.5 s after a canopy breach, 0.4 s after the module loses power | 6.5, 0.4 | B | LifeSupportComponent template |
| Capacitor recharge (numbers) | lo 0, p 1.1 on every channel (rate = hi x s^1.1) | 0, 1.1 | B | distributor record |
| Pips (numbers) | start 2/2/2, at most 4 a channel, a press moves 1, the reset leaves no unassigned pips | half pips 4/4/4, 8, 2, 0 | B | PowerDistributorModule template |
| Simulation rate | 60 Hz, at most 4 sub-steps a frame, in every location | 60, 4 | B | PhysicsWorld template |
| Mass lock factor | the Sidewinder's mass-lock factor 6 (stations' components 1, ranges 5,000 / 20,000 m by shape) | 6 | B (rule open) | MassLockComponent |
| FA time constant (correction, N1b-1) | T = max(0.5, D x max(0, 4/3 x min(1, s/Vfull) - 1/3)); D is the boost record's field 5 while boosting | 0.5, 4/3, 1/3, 1 | C | 0x142E6E960 |
| Linear jerk limit (correction, N1b-1) | the limit's memory (+0x630) is written only when it runs (FA on, not boosting), after the clamp, before the speed guard; a boost leaves it as it was (why braking after a boost ramps up) | - | C | 0x142E6E960 |
| Full-forward brake (settles the flag, N1b-1) | lower forward limit = 0.25 x block 0x20 when the forward demand is 0.999 or more; flag +0x5E2 set to 1 by the constructor, no other writer found | 0.25, 0.999 | C | 0x142E6AF90, 0x142E32DD0 |
| Reversal mode 1 (detail, N1b-1) | alpha x (1 + block 0xA8..) where the stick is over 0.2 and sign(target) x sign(omega) <= 0 (omega = 0 counts as +); applied after the jerk limit; +0x640 keeps the unmultiplied alpha | 0.2, factors 1 | C + B | 0x142E6E960 |
| Boost limits (detail, N1b-1) | while firing, the upper limits x field 4, lateral and vertical also x 0x30 / 0x34; lower limits unchanged | x4 | C + B | 0x142E6E960 |
| Turn rates, accelerations (replaces the block table's coriolis values) | pitch 35 to 43 by the ENG pips, yaw 17, roll 110 deg/s; forward 45, reverse 30, lateral 30 m/s^2 at every pip setting (session 2's fit of 43.42 at 0 pips is not supported by the records; neither value passes the strict gate) | - | B | performance record |
| HUD speed readout (instrument, N2) | the number is rounded to the nearest m/s and redrawn every other 60 Hz frame (the picture itself changes every frame) | 30 /s | D | session 2: the four steady speeds read 161, 101, 97, 221 against the records' 160.86, 100.54, 96.52, 221.18; the boost climb reads in pairs (`hud_update_rate.py`) |
| The keys' delays (input, N2) | the game acts on the throttle keys W and S 67-92 ms after the key, on a thruster key's release 55-59 ms, on the boost 26-32 ms, on X and a thruster key's press 13-22 ms; the rule behind it is not found. The keyboard throttle's rate does not show in the speed (0.4 /s to instant replay the same: the jerk and acceleration limits bind) | 0.526 /s kept | D | N2 and its review, sessions 1 and 2 (`throttle_scan`, `phase_shifts`) |
| Keyboard throttle node (N2b) | each frame the throttle T += that frame's increment from the node before it; crossing zero sets T = 0 and holds it until the increment drops below FLT_EPSILON (the key let go) or the node's hold time passes; a set input or SetSpeedZero sets T; clamped to [-1, 1] ([0, 1] when forward-only); written the same frame. No ramp, no hold before moving | FLT_EPSILON, 1, 0.5 | C | 0x1408F3500 |
| Input graph node output (N2b) | a node writes its output only if larger in magnitude than the value already there this frame | - | C | 0x1408F3500, 0x1408F3CA0 |
| Smoother node (N2b; its type inferred) | mode 0: the mean of the input over the last W s; mode 1: moves toward the input by at most dt/W a frame; the same for press and release | W from data | C (code); whether the ship's keys use it is not known | 0x1408F3CA0 |
| Demands to the ship (N2b) | PlayerShipControl hands ForwardReverseThrust, LateralThrust, VerticalThrust on unchanged (clamped to +-1, 0 where a direction is blocked) and UseBoostJuice to the boost byte +0x9E; the flight controller copies them each frame | - | C | 0x142DF73E0, 0x142E04A00, 0x142E4A820 |
| The keys' delays (N2b, update) | the keys reach those nodes through a graph loaded as data (not in the exe); its nodes and their numbers are not read, so the delays still have no rule | - | open | `re/elite/notes/sidewinder_input.md` |
| S identified (N3) | A50/A60's object S is the ship's ShipPlanetProximityComponent's interface (component +0x58): +0x28 mode (+0x190), +0x30 target (kind 2), +0x38 w | - | C | 0x1453183E0, 0x142C604A0, 0x1405C3E50, 0x142C98630 |
| Orbital zone radii (N3) | per body of kind 2 passing body virtual +0x50(2): outer = R Z4C + Z50; inner = Z58 + (R + max(h, Z5C)) Z54; counts inside Z60 x outer and R x Z64; Z = the template of the type-0x145F05C68 system component | Z not read | C (h = the terrain's top, inferred) | 0x142CA3520, 0x142C9A3D0, 0x143CF5B90, 0x143CF0290, 0x143CDAB20 |
| Orbital mode and target (N3) | each frame the largest k = clamp(1 - (d - R)/(Z60 x outer - R)) wins; mode 2 if d < inner, else 1 if d < outer (else the mode an earlier winner set) | - | C | 0x142C88F10 |
| Orbital cruise blend w (N3) | w = 0 in mode 0; else 1 - clamp((d^2 - r^2)/(outer^2 - r^2)), r = inner + 3,000 m (a gate at S+0x98 is not read) | 3000 | C | 0x142C99400 |
| Full-throttle brake's flag (N3; corrects the row "Full-forward brake (settles the flag, N1b-1)" above: the flag is not always 1) | the constructor sets +0x5E2 to 1; in supercruise, while the +0x128 object is missing or inactive, it is set to (w <= 0.01); the quarter brake needs it; it stays as last set in normal flight | 0.01 | C | 0x142EA9810, 0x142E787C0 |
| Cap heading factor (N3) | in mode 1 with a target: m x (1 + 1.5 g), g = 1 - clamp((theta - 2)/4), theta = abs(asin(row+0x20 . u)) in degrees | 2, 6, 2.5 | C + B (the axis open) | 0x142EA9810 |
| Safe stop by mode (N3; corrects A60's) | no target or v >= z + 1,000: emergency; mode 0: nothing; mode 1: emergency if v > 2(z + (f - z) w); mode 2: glide if v < f, else emergency (a per-body override of f is not read) | 1000, 2.0 | C | 0x142CAE4A0 |
| Gear and scoop speed limit (N3) | A = record +0x3EC while the gear is asked down (the request byte +0xBB, at once), B = +0x3E8 while the hatch is open (state 3); top and boost speed of both blocks x min(A, B), nothing else; with supercruise active the factor goes into the cruise block's own k instead (not built) | 0.5, 0.5 | C + B | 0x14307F8A0, 0x142A69A40, 0x142E78C10, 0x142E780D0, 0x142E86E20, 0x141B2DA50, 0x141B2D8F0 |
| Boost with deployables (N3) | allowed; its only refusals are the engines off and too little energy | - | C | 0x142E4A820 |
| Not found (N3) | Z's seven numbers, S+0x98's gate, S+0x68, what makes the gear and scoop retract on their own | - | open | `re/elite/notes/sidewinder_orbital_cruise.md` |
| Gear and scoop in supercruise (N4) | with supercruise active and k != 1: k = m x min(A, B) goes into accel mode 2 of the current block only (top speed, forward and lateral acceleration, the reverse acceleration's first term); the full-power block, the boost speed and +0x688 are untouched; the turn curve's x is v / (cap x k / m) | 0.5, 0.5 | C | 0x142E86E20, 0x141B2D8F0, 0x142E79C60, 0x142E6E960 |
| Supercruise time constant (N4) | T = max(0.5, D max(0, 4/3 min(1, v/Vsc) - 1/3)); the full-power block's top in supercruise is the raw record's Vsc, since mode 2 scales only the current block; T stays 0.5 s up to 0.625 Vsc | 0.5, 4/3, 1/3, D = 1 | C | 0x142E6E960, 0x142E86E20 |

## The heat scale (UI4c, 2026-11-23)

| Rule | Formula (our words) | Constants | Grade | Source |
|---|---|---|---|---|
| HUD heat percent (settles "what the HUD % is" in A20) | label = int(f / 1.5 x 100 + 0.5) %, f = heat / capacity; the gauge gets f / 3.5 with marks 1.5/3.5 and 2.5/3.5 (the movie names them the module-damage and hull-damage marks); 100 % is where `$HeatWarning100;` fires, full scale 233 %. The movie redraws the label only when the sent value has moved 0.01 (2.33 points), so the label lags the heat | 1.5, 3.5, 2.5, 0.01 | A | FUN_142d6d3e0; the movie's `Heat.as`; `re/elite/notes/ui_messages_hud.md` |
| Heat damage | from f 2.5 (HUD 167 %), at its most from f 3.5 (233 %); the record's rates 0.5 / 1.5 have no known unit (receiver not traced), so the mod keeps 0 to 0.01 of the hull a second | 2.5, 3.5 | B (thresholds), E (rates) | 0x142DC1940, heat record +0xF4, +0xF8 |
| Idle load (refit) | holds session 2's idle 36 % (f 0.54): 1.18 + 16.97 x 0.54^2 = 6.128 heat/s; was 3.379 under f x 100. The label's 2.33-point step means "36" bounds the true idle only loosely; Elite's 34 % just after loading (sessions 1, 4, ui1) is the label's last redraw before a balance point just under 36.0 | 6.128 | E | s09 |

Checks against session 2 under the new scale: a boost from idle rises 4.76 %/s (measured about 4.7) and peaks at 44.3 %
(measured 44; 49 under the old reading). Open, new with the refit and not tuned: after a boost the heat falls faster than
Elite's (Elite's true heat, worked back from its label's redraws, is 41.3 % at +10.3 s and 39.0 % at +16.0 s after the
boost key; the model reaches those at about +6.1 s and +10.2 s; the old fit under the old reading was 20 to 25 % slower
than Elite). The time constant at idle is 7.6 s against Elite's about 10 s: that points at the radiator's stand-ins (its
least rate 1.18, a field the exe's rate code does not read; its multiplier k; the curve's top), which an idle decay
flown at rest would settle.
