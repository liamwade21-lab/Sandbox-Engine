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
| 0x1C, 0x20, 0x24 | forward, reverse, lateral/vertical acceleration (m/s^2) | 44.39, 29.96, 29.96 | **provisional**: EDSY's `fwdacc`, `revacc`, `latacc` (E8, a hint), until measured (D, step 0.9) |
| 0x44..0x4C | angular accelerations (deg/s^2) | pitch 220, yaw 110, roll 240 | **provisional**: EDSY's `pitchacc`, `yawacc`, `rollacc` (E8), until measured |
| 0x2C, 0x5C..0x64 | the flight assist's linear and angular time constants (D, R) | not known | measured (D) in step 0.9; until then the code's own floor (T 0.5 s) stands |
| curves 0x68.., 0x78.., 0x88.. | the blue zone's (a, b, p) per curve | not known | measured (D); EDSY's `minpitch` 34 (a hint: the turn rate at the curve's ends) |
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
| Turn rates | W_axis = block 0x38 (x) / 0x40 (y) / 0x3C (z) in deg/s x pi/180 x g_rot x speed factor (x, y: curve 0x78..; z: curve 0x88..); pitch/yaw/roll naming of the axes inferred | pi/180 | C | 0x142E6E960 |
| Angular acceleration limit | A_axis = block 0x44 / 0x48 / 0x4C in deg/s^2 x pi/180 x g_rot x curve 0x68.. | - | C | 0x142E6E960 |
| Rotation (FA on) | alpha = (clamp(stick x W + corrections, +-W) - omega) / tau_r, tau_r = max(dt, R x max(0, 10/9 x s / Vtop - 1/9)), R = block 0x5C..0x64; clamp +-A; angular jerk limit block 0x50..0x58 (deg/s^3) | 10/9, 1/9 | C | 0x142E6E960 |
| Rotation (FA off) | alpha = A x stick per axis; none on an axis already at W turning the stick's way | - | C | 0x142E6E960 |
| Rotation reversal modes | block 0xA4 = 1: alpha x (1 + block 0xA8..) where |stick| > 0.2 opposes the spin; = 2: spin scaled by block 0xB4.. there | 0.2 | C | 0x142E6E960 |
| Blue zone (speed curve) | x = clamp(s / V, 0, 1); x <= p: a + (1 - a) x / p; else b + (1 - b)(1 - (x - p)/(1 - p)); (a, b, p) per curve from the block | - | C | 0x142E50650 |
| Turning while/after boost | within 3 t s after a boost (and during it): turn factor 0.1 t + block 0x84, z factor block 0x94, accel factor block 0x74 | 3.0, 0.1 (data) | C | 0x142E6E960 |
| Thruster mass curve | x = clamp((m - mmin)/(mmax - mmin), 0, 1); e = log((kopt - kmin)/(kmax - kmin)) / log(1 - (mopt - mmin)/(mmax - mmin)) (e = 1 if that share >= 1); k = kmin + (kmax - kmin)(1 - x)^e | none in exe (curves are module data) | C | 0x142CAC400 |
| Mass curve use | 9 curves in the thruster data: speeds (0x00, 0x0C), accelerations (0x1C..0x24), turn rates (0x38..0x40), the three boost curve values, boost accel multipliers, and one added to the boost ramp time | - | C | 0x142EBDF60, 0x141DFC6E0 |
| Pip blend (ENG) | current block = base + (full-power - base) x t field by field (negative full-power values skipped); t = ENG share (inferred); full-power block uses t = 1 | - | C (t's meaning inferred) | 0x142E86E20, 0x141B2E310 |
| Speed multipliers | top and boost speed x k, k = min(A, B) x C x module factor x FSD-object factor; boost speed also x module factor and x boost field 7 | - | C (A, B, C unnamed) | 0x142E86E20 |
| Boost | states idle / warm-up (field 0 s) / firing (field 1 s) / cool-down (field 2 s); energy cost field 3; capacitor drain field 6 per s; stick pushed to full forward over block 0x10 s | - | C | 0x142E4A820 |
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
