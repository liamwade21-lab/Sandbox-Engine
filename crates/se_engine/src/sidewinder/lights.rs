//! The ship's lights (ARCHITECTURE.md A6): which of the state machines' point and spot lights are on and how bright, written
//! into the lights buffer (`ShipLights`, A6's contract), Elite's exposure inputs (`ShipScene`) and the heat effect's glow
//! (`ShipGlow`), each in place and only when a value changes (every frame only while an animator moves). Nothing here runs
//! before the first spawn: the systems wait for the ship's files (`LightList` is made with the meshes, `draw::make`).
//!
//! The list (`LightList`): one slot per light activity and bone (`Model!Bone`: a light at each); a slot keeps its place
//! whatever its level. Which lights a surface takes is Elite's light gather (FUN_1435b9020, the job FUN_1435ac1c0 builds per
//! view from the view's flags +0x688): a view with bit 3 set (FUN_142855570) takes ONLY the lights whose light record's +0x36
//! is set, which FUN_1435cb790 copies from the data's byte +0x5a (`Light::bytes_88[2]`; set on the cockpit's lamps but the red
//! warning lamp, and on the vent lights), matched by the record's +0x18 handle (the data's +0xd8 / +0xe0 name, empty on every
//! light of the ship, so the match lets every flagged light through); a view without it takes every light (both after the
//! layer mask, light +0x14 against the view's +0xa0, not traced). The seat's view is taken to be the flagged one (Elite draws
//! the cockpit as its own scene; which view sets bit 3 was not traced: a reading), the outside's keeps the hull's lights (the
//! unflagged rule would give it the cockpit's too, behind the untraced layer mask: an open item). So the slots go: the hull's
//! unflagged lights, the hull's flagged (the vents), the cockpit's flagged, the cockpit's unflagged (the red warning lamp),
//! each group in the machines', states' and activities' file order; the outside's materials take the hull's range, the
//! seat's the flagged range (`LightList::range`), each contiguous.
//!
//! A light sits at its bone's origin, a spot pointing along the bone's local +Z (ours, from the data: the headlights' bones'
//! +Z runs along the nose, the bottom spots' and the back spots' at the hull; the exe's choice of axis was not traced) and
//! pulled back along it by inner fraction / tan(cone / 2), its colour times (1 + that)^2 (FUN_143590b40). A light shines only
//! while its machine's state (`Parts::states`) is the activity's state: leaving the state stops the activity (vtable slot 13,
//! FUN_1435bc330, the spot's FUN_1435bc470, unsubscribes the per-frame job that Start, slot 12, subscribed; no fade-out runs
//! after it in the code at hand; re/elite/notes/agent_csm.md 5b), so the light gives none at once and its animator stands still until the state comes back (whether Elite
//! makes the activity anew then, restarting the animator, was not traced). In its state its level is the intensity
//! animator's (`Animator`, FUN_143598ba0 to the letter): the record's off and on levels, its bool parameters switching it, its
//! float parameters' product (rate-limited by the fades, raised to the exponent) scaling it, its fades, hold, timed blink or
//! step pattern, and jitter; times the exposure factor 2^(clamp(scene_ev + bias, min, max) - 3) for a light with exposure on
//! (FUN_1435a5500, FUN_142832db0), which the shader applies from each slot's terms (`Lamp::exposure_terms`, `ShipScene`), so
//! it follows the exposure the frame is drawn with (`super::exposure`).
//!
//! The parameters (what feeds each, in the order: the notes, the exe, else the data's default as a labelled stand-in):
//! - `Option_HeadlightsEnabled` (bool, the headlights): the pilot's lights switch (`Pilot::lights`; the name and the
//!   headlights' spot activity, re/elite/notes/agent_paint.md section 6).
//! - `Option_LandingGearDeployed` (bool, the two beacons' pairs): the gear as the parts are posed (`Parts::gear_down`).
//! - `Warning_Overheating` (bool, the red warning lamp): heat / capacity at least 1.5 (FUN_1441988d0 compares the heat
//!   object's +0x20 with 1.5 at 0x144dedfd8).
//! - `Warning_DangerProximity` (bool, the red warning lamp, which needs both: its bool mode is 0, all true): no writer
//!   found (FUN_142ea6b00, FUN_142e64a20 only bind it): `DANGER_PROXIMITY`, false, a stand-in.
//! - `HeatRadiator_VisualActiveDissipation` (float, the vent lights): FUN_144399d90 via FUN_144398f40: clamp((f - f0) /
//!   (1 - f0), 0, 1) of the heat fraction f, 0 while the radiator is off (silent running); f0 the radiator record's +0x18
//!   (`elite_sim::heat::DISSIPATE_F0`, 0). Its second condition (vtable +0x70 of the component's +0x158) and the heat
//!   sink's term (a ship without one has none) are taken as passing.
//! - `HeatRadiator_VisualPassiveDissipation` (float, the heat effect's glow): the same function's clamp(f / f1, 0, 1), f1
//!   the record's +0x1c (`DISSIPATE_F1`, 1), 0 while the radiator is off. Its `_OverRide` (used when at least 0) is never
//!   set by the mod.
//! - `cockpitAmberLightIntensity` (float): created with 1.0 by FUN_144733e70; its writer was not found: `AMBER`, 1.0.
//! - `OverrideBrightnessScale` (float) and the GUI lamps' `OverrideColour_*`: FUN_1430dbae0 works them out from the HUD's
//!   panel colours, which the mod has none of: the brightness `OVERRIDE_BRIGHTNESS`, 1.0, and the GUI lamps keep the
//!   record's colour, (0, 0, 0): those five lamps stay dark (an open item).
//! - The lighting component's brightness (FUN_1435a5500: the colour times vtable +0x80 of the light's owner's lighting
//!   object, for every light without data +0x14a): its writer was not traced (`lighting_brightness`: the hull's 1.0, the
//!   cockpit's a measured stand-in, its doc).
//!
//! **Elite's exposure inputs (A6's known gap; the user's decision: the mod's own exposure stands in).** Elite's lit output is
//! in its renderer's units; this game's light record (the star, the sky) is in its own. Every stand-in is worked out from
//! this game's exposure through one mapping:
//! 1. `ELITE_UNIT` (U): this game's light units per Elite renderer unit, one constant for everything Elite emits or lights
//!    (lamps and glows add into the same target in Elite). A lamp's light is colour x level x factor x U, so a hull lamp
//!    (exposure off) keeps its ratio to the sunlight whatever the exposure. U is bracketed by a measurement (its doc).
//! 2. The game's exposure E = 2^(the main camera's ColorGrading exposure, which holds `ExposureComp`'s stops), and in photo
//!    mode 2 times 2^-(the adapted compensation, on the GPU only: `super::exposure` copies it to the ship's shaders); the
//!    scene luminance it maps to the auto exposure's target is L = AutoExpTargetBright / E.
//! 3. `scene_ev` = log2(L / U): the scene's luminance in Elite's units, as an EV (the manager's +0x6c read as the average
//!    luminance's log, agent_paint.md "sceneValue": a reading). It only moves the cockpit lamps inside their clamps (8..8.5
//!    or 8.5..9: the factor stays within 2^5..2^6), so no screenshot can fix it any closer.
//! 4. `luminance` = 2^scene_ev (the rescaling's texel; x 0.125 in the shader, which is the lamps' "- 3") and
//!    `emissive_multiplier` = 2^(scene_ev - 3), so a rescaled glow is e x U x L / U x 2^(bias - 3): steady on screen.
//! 5. `self_illum_scale` = U x `SELF_ILLUM` (gGlobalSelfIllumScale, not found: 1.0), so a glow keeps its ratio to the lamps.
//!
//! The CPU works out steps 3 and 4 for the camera's exposure alone (`scene_inputs`, `ShipScene`'s inputs); the shader adds the
//! adapted compensation's stops to `scene_ev` and multiplies `luminance` and `emissive_multiplier` by 2^them while photo mode
//! 2 is on (`sidewinder_lit.wesl` `adapted`).

use bevy::math::{UVec4, Vec3, Vec4};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::render::view::ColorGrading;
use se_core::elite_ship::{parse_pattern, Activity, KnownActivity, Light, LightAnimator};

use super::assets::{MachineOwner, ShipRig};
use super::material::{HeatGlow, SceneExposure, ShipGlow, ShipLight, ShipLights, ShipScene, MAX_SHIP_LIGHTS};
use super::parts::Parts;
use super::pilot::Pilot;
use super::SidewinderAssets;

/// U: this game's light units per Elite renderer unit (the module's notes, step 1), a stand-in for the unfound exposure
/// inputs, grade E: a round number that a measurement brackets, nothing in Elite's code. (Elite's gHDRScalingWrite is also
/// 1/256, but that is only the scale its HDR target is stored at, undone on reading: no reason for this value.) The
/// measurement, on the green navigation light, the one lamp whose light falls on hull out of the sun in both pictures: the
/// user's top view `7.webp` against the game's (the Moon by day, the same framing), each picture's pixels made linear and
/// divided by its own sunlit white hull (sRGB 148 in Elite's, 178 in the game's). The hull lit green above 0.1, 0.2, 0.4 and
/// 0.8 of that white covers 0.22, 0.16, 0.12, 0.076 m^2 in Elite's picture and 0.30, 0.17, 0.10, 0.044 m^2 in the game's at
/// this U; matching the areas threshold by threshold puts Elite's lamp at 0.65, 0.89, 1.16 and 1.47 times the game's (the
/// spread is the two tone curves' and Elite's bloom), geometric mean 0.99: U holds within 0.65 to 1.56 times. It is
/// relative to the sunlit hull, so it also depends on how bright the star was in `7.webp` (not known) against the Moon's sun.
pub const ELITE_UNIT: f32 = 1.0 / 256.0;
/// gGlobalSelfIllumScale: not found; 1.0 (a stand-in).
pub const SELF_ILLUM: f32 = 1.0;
/// The hull's lighting component's brightness, which every light of its machines is multiplied by (FUN_1435a5500: vtable +0x80
/// of the owner's lighting object): its writer was not traced; 1.0, a stand-in (the hull's lamps match `4.webp` and `7.webp`
/// with it, within U's bracket).
pub const HULL_LIGHTING_BRIGHTNESS: f32 = 1.0;
/// The cockpit's lighting component's brightness (the same multiplier for the cockpit machines' lights; the cockpit is an
/// owner of its own), grade D: a measured stand-in for the lighting brightness whose writer was not found (the exe gives
/// the cockpit lamps' exposure factor, 2^5 to 2^5.5, exactly: FUN_1435a5500, FUN_142832db0; their light was ~20 times too
/// strong with 1.0). Measured from the seat at night, where only the cockpit's lamps light the console (with them off it is
/// black) and the red canopy lamps glow by themselves (self-illumination, not this): the user's `1.webp` (in space, beside a
/// station and a planet) against the game's picture from the same eye at 3440x1440 (`.optimize/scratch/run/t4a4_measure.py`,
/// both at 2000 px wide, linear luminance over the left red lamp's median, 0.086 in Elite's, 0.101 in the game's), the
/// regions placed about the radar's middle in each: the radar's plinth 0.021 against the game's 0.88 (42 times), the radar's
/// lower half 0.028 against 0.86 (31), the left console 0.0010 against 0.0147 (15), the right 0.0015 against 0.0168 (11).
/// This is 1 / their geometric mean, 21.6; the regions spread it by a factor of 2 either way, and Elite's darkest values are
/// a code or two of its 8-bit picture. Elite's console may also take light from the HUD's holograms or the planet (the
/// plinth lies under the radar's hologram), which would make the true factor smaller still.
pub const COCKPIT_LIGHTING_BRIGHTNESS: f32 = 1.0 / 21.6;

/// The lighting component's brightness a light of `owner`'s machines is multiplied by (the list holds only the hull's and the
/// cockpit's).
pub fn lighting_brightness(owner: MachineOwner) -> f32 {
    if owner == MachineOwner::Cockpit {
        COCKPIT_LIGHTING_BRIGHTNESS
    } else {
        HULL_LIGHTING_BRIGHTNESS
    }
}
/// `cockpitAmberLightIntensity`: its creation value (FUN_144733e70); its writer was not found.
pub const AMBER: f32 = 1.0;
/// `OverrideBrightnessScale`: worked out from the HUD's panel colours (FUN_1430dbae0), which the mod hasn't; 1.0.
pub const OVERRIDE_BRIGHTNESS: f32 = 1.0;
/// `Warning_DangerProximity`: no writer found; false.
pub const DANGER_PROXIMITY: bool = false;
/// `Warning_Overheating` is heat / capacity at least this (FUN_1441988d0, 0x144dedfd8).
pub const OVERHEAT_WARNING: f64 = 1.5;
/// A spot light's IES scale divisor, data +0x1a8 (1.0 on the ship's headlights): K = (pi / cone) / max(it, 1).
const IES_DIVISOR: f32 = 1.0;

// ---- The intensity animator (FUN_1435ea060 builds it, FUN_143598ba0 steps it) ----

/// A small random source for the animator's draws (FUN_14055c4d0 is the exe's; this is ours: uniform, seeded per slot).
#[derive(Clone, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9) | 1)
    }

    /// Uniform in [lo, hi].
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        lo + (hi - lo) * (self.0 as f32 / u32::MAX as f32)
    }
}

/// One light's animator state, the fields FUN_143598ba0 keeps (float indices of the light record's +0xf8 block in brackets).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animator {
    /// [0x0f] the blink's state; [0x10] its timer; [0x11] its start; [0x14] 1 / the rate; [0x15], [0x16] the on and off
    /// phases; [0x17] the random extra's bound, [0x18] its draw.
    blink_on: bool,
    timer: f32,
    start: f32,
    rate_inv: f32,
    on_len: f32,
    off_len: f32,
    extra: f32,
    extra_draw: f32,
    /// [0x0e] the hold's countdown; [0x25] the fade; [0x26] the rate-limited parameters' product. All start at 0, as
    /// FUN_1435ea060 leaves them (where the block is first zeroed was not traced: an open item).
    countdown: f32,
    fade: f32,
    slewed: f32,
    /// [0x34] the jitter's value, [0x37] its timer, [0x38] its period (0: none).
    jitter: f32,
    jitter_timer: f32,
    jitter_period: f32,
}

impl Animator {
    /// As FUN_1435ea060 builds it from the record (the jitter runs when the creation's draw in [0, 1) is at most +0xa0).
    pub fn new(a: &LightAnimator, rng: &mut Rng) -> Animator {
        let mut s = Animator::default();
        let rate = a.blink_rate as f32;
        if rate <= 0.0 {
            s.blink_on = true;
        } else {
            let inv = 1.0 / rate;
            s.rate_inv = inv;
            s.start = inv * a.blink_start as f32;
            if a.blink_extra > 0.0 {
                s.extra = a.blink_extra as f32 * inv;
            }
            let duty = a.blink_duty as f32;
            s.on_len = duty * (inv + inv);
            s.off_len = (1.0 - duty) * (inv + inv);
        }
        // (Spot lights take their runtime byte +0x28c instead of the draw, its writer not traced: taken as set.)
        let roll = rng.range(0.0, 1.0);
        if a.jitter_rate_hz > 0.0 && a.jitter_chance.is_none_or(|c| roll <= c as f32) {
            let period = 1.0 / a.jitter_rate_hz as f32;
            s.jitter_period = a.jitter_spread as f32 * rng.range(-period, period) + period;
        }
        s
    }

    /// One frame of `dt` s (FUN_143598ba0): `on` is the animator's running byte (+0x30; `update` steps a lamp only while its
    /// machine is in its state, so always true there), `bools`
    /// and `floats` its parameters' values in its lists' order, `steps` its pattern (empty: the timed blink), `scale` the
    /// exposure factor. The level out: max(0, level2 x jitter + off + (on - off) x level) x scale.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, a: &LightAnimator, steps: &[bool], dt: f32, on: bool, bools: &[bool], floats: &[f32], levels: (f32, f32), scale: f32, rng: &mut Rng) -> f32 {
        let enabled = match a.bool_mode {
            0 => on && bools.iter().all(|&b| b),
            1 => on && bools.iter().any(|&b| b),
            2 => on && !bools.iter().any(|&b| b),
            _ => on,
        };
        self.countdown = if enabled { a.hold_s as f32 } else { self.countdown - dt };
        if steps.is_empty() {
            if self.rate_inv > 0.0 {
                let phase = if self.blink_on { self.on_len } else { self.off_len };
                let mut base = self.timer;
                if phase + self.extra_draw + self.start <= self.timer {
                    let next = if self.blink_on { self.off_len } else { self.on_len };
                    base = self.start;
                    if next > 0.0 {
                        self.blink_on = !self.blink_on;
                        // (The draw's upper bound is not in the decompile; the lower is -extra: taken as symmetric.)
                        self.extra_draw = rng.range(-self.extra, self.extra);
                    }
                }
                self.timer = base + dt;
            }
        } else {
            let cycle = a.pattern_cycle_s as f32;
            let n = steps.len() as f32;
            let at = ((self.timer - self.start) / cycle).max(0.0) * n;
            self.blink_on = steps[at.min(n - 1.0) as usize];
            self.timer += dt;
            if cycle + self.start < self.timer {
                self.timer -= cycle;
            }
        }
        let blink = if !self.blink_on && a.fade_out_s <= 0.0 { 0.0 } else { 1.0 };
        self.jitter_timer += dt;
        if self.jitter_period > 0.0 && self.jitter_period < self.jitter_timer {
            self.jitter_timer = 0.0;
            self.jitter = rng.range(-(a.jitter_below as f32), a.jitter_above as f32);
        }
        let (fade_in, fade_out) = (a.fade_in_s as f32, a.fade_out_s as f32);
        self.fade = if self.countdown < 0.0 || !self.blink_on {
            (self.fade - if fade_out > 0.0 { dt / fade_out } else { 1.0 }).max(0.0)
        } else {
            (self.fade + if fade_in > 0.0 { dt / fade_in } else { 1.0 }).min(1.0)
        };
        let product: f32 = floats.iter().product();
        let up = if fade_in > 0.0 { self.slewed + dt / fade_in } else { f32::MAX };
        let down = if fade_out > 0.0 { self.slewed - dt / fade_out } else { f32::MIN };
        self.slewed = product.clamp(down, up.max(down));
        let level = self.fade * blink * self.slewed.powf(a.exponent as f32);
        let level2 = if a.hard { (scale > 0.0) as u8 as f32 } else { level };
        let (on_level, off_level) = levels;
        (level2 * self.jitter + off_level + (on_level - off_level) * level).max(0.0) * scale
    }

    /// Does the level move on its own now (a blink or pattern, a jitter, a fade or a product still on its way)?
    pub fn moving(&self, a: &LightAnimator, has_pattern: bool) -> bool {
        has_pattern || self.rate_inv > 0.0 || (self.jitter_period > 0.0 && self.fade > 0.0) || (self.fade > 0.0 && self.fade < 1.0) || (self.countdown > 0.0 && self.countdown < a.hold_s as f32)
    }
}

/// The exposure factor a light's level is multiplied by on the CPU (FUN_1435a5500 with FUN_142832db0): with exposure on,
/// 2^(clamp(scene_ev + bias, min, max) - 3), else 1.
pub fn exposure_factor(l: &Light, scene_ev: f32) -> f32 {
    if !l.exposure_enable {
        return 1.0;
    }
    let [lo, hi] = l.floats_112.map(|x| x as f32);
    let x = scene_ev + l.exposure_bias as f32;
    let x = if lo <= x && x <= hi { x } else if x < lo { lo } else { hi };
    (x - 3.0).exp2()
}

/// Elite's exposure inputs from this game's exposure (the module's notes): `exposure` the main camera's ColorGrading
/// exposure in stops, `target` the auto exposure's target brightness.
pub fn scene_inputs(exposure: f32, target: f32) -> SceneExposure {
    let scene_ev = (target.max(1e-6) / (exposure.exp2() * ELITE_UNIT)).log2();
    SceneExposure { scene_ev, self_illum_scale: ELITE_UNIT * SELF_ILLUM, emissive_multiplier: (scene_ev - 3.0).exp2(), luminance: scene_ev.exp2() }
}

// ---- The list ----

/// One slot of the list: a light activity at one of its bones.
#[derive(Clone, Debug)]
pub struct Lamp {
    pub machine: usize,
    pub owner: MachineOwner,
    pub state: String,
    pub bone: String,
    pub spot: bool,
    pub light: Light,
    pub animator: LightAnimator,
    steps: Vec<bool>,
    /// In the ship's space: the bone's place and +Z axis (None when no node has the bone: the slot stays dark).
    place: Option<(Vec3, Vec3)>,
}

impl Lamp {
    /// The light's name as its holder gives it.
    pub fn holder(&self) -> &str {
        self.light.holder.as_deref().unwrap_or("(no holder)")
    }

    /// Is the light gathered by the flagged view (the record's +0x36, from the data's +0x5a: the module's notes)?
    pub fn flagged(&self) -> bool {
        self.light.bytes_88[2] != 0
    }

    /// The slot's exposure term for the shader (`ShipScene`): x 1 with the term on (data +0x68), y the bias (+0x6c), z and w
    /// the clamp (+0x70, +0x74); all 0 with it off (a factor of 1).
    pub fn exposure_terms(&self) -> Vec4 {
        let l = &self.light;
        if !l.exposure_enable {
            return Vec4::ZERO;
        }
        Vec4::new(1.0, l.exposure_bias as f32, l.floats_112[0] as f32, l.floats_112[1] as f32)
    }

    /// The buffer's record at `level` (the animator's intensity, without the exposure factor, which the shader applies): the
    /// colour x the owner's lighting brightness x `level` x U.
    pub fn record(&self, level: f32) -> ShipLight {
        let Some((pos, axis)) = self.place else { return ShipLight::default() };
        let l = &self.light;
        let colour = Vec3::from_array(l.colour.map(|c| c as f32)) * (lighting_brightness(self.owner) * level * ELITE_UNIT);
        let (range, size) = (l.range_m as f32, l.source_size as f32);
        if !self.spot {
            let specular = l.specular_scale.unwrap_or(1.0).clamp(0.0, 1.0) as f32;
            return ShipLight { pos_range: pos.extend(range), colour: colour.extend(size), dir_cos: Vec4::new(0.0, 0.0, 0.0, -2.0), spot: Vec4::new(0.0, 0.0, 0.0, specular) };
        }
        let cone = (l.cone_deg.unwrap_or(0.0) as f32).to_radians().max(0.001);
        let ies = l.use_ies.unwrap_or(false);
        let inner = if ies && l.inner_from_cone.unwrap_or(false) { 0.9 * cone } else { (l.inner_deg.unwrap_or(0.0) as f32).to_radians() };
        let inner = inner.clamp(0.001, cone);
        // (FUN_143590b40: the apex pulled back by inner fraction / tan(cone / 2), the colour times (1 + that)^2; the spot's
        // specular scale is the base constructor's 1.0, FUN_143586230.)
        let pull = l.inner_fraction.unwrap_or(0.0) as f32 / (cone * 0.5).tan();
        ShipLight {
            pos_range: (pos - axis * pull).extend(range),
            colour: (colour * (1.0 + pull) * (1.0 + pull)).extend(size),
            dir_cos: axis.extend((cone * 0.5).cos()),
            spot: Vec4::new((inner * 0.5).cos(), std::f32::consts::PI / cone / IES_DIVISOR.max(1.0), if ies { 1.0 } else { 0.0 }, 1.0),
        }
    }
}

/// The ship's light slots in the buffer's order (the module's notes), and the hull's share (A6). Made once with the ship's
/// meshes.
#[derive(Resource, Clone, Debug)]
pub struct LightList {
    pub lamps: Vec<Lamp>,
    /// The hull's lights are `lamps[..hull]`.
    pub hull: usize,
}

impl LightList {
    /// Every light activity of the hull's machines, then of the cockpit's, at each of its bones, placed with `values`; the
    /// hull's unflagged lights before its flagged ones, the cockpit's flagged before its unflagged, so the flagged are one run.
    pub fn new(rig: &ShipRig, values: &se_core::elite_rig::Values) -> Result<LightList, String> {
        let mut lamps = Vec::new();
        let mut hull = 0;
        for owner in [MachineOwner::Hull, MachineOwner::Cockpit] {
            let start = lamps.len();
            for (i, m) in rig.ship.machines.iter().enumerate().filter(|(i, _)| rig.machine_owner(*i) == owner) {
                for s in &m.states {
                    for a in &s.activities {
                        let (light, spot) = match a {
                            Activity::Known(KnownActivity::PointLightActivity(l)) => (l, false),
                            Activity::Known(KnownActivity::SpotLightActivity(l)) => (l, true),
                            _ => continue,
                        };
                        let named = |e: String| format!("machine {}, state {}, light {}: {e}", m.name, s.name, light.holder.as_deref().unwrap_or("?"));
                        let animator = light.animator.clone().ok_or_else(|| named("no animator (ship.json is older than the lights)".into()))?;
                        let steps = animator.pattern.as_deref().map(parse_pattern).transpose().map_err(named)?.unwrap_or_default();
                        for bone in &light.bones {
                            lamps.push(Lamp { machine: i, owner, state: s.name.clone(), bone: bone.clone(), spot, light: light.clone(), animator: animator.clone(), steps: steps.clone(), place: None });
                        }
                    }
                }
            }
            // (A stable sort: each group keeps the file's order.)
            lamps[start..].sort_by_key(|l| l.flagged() == (owner == MachineOwner::Hull));
            if owner == MachineOwner::Hull {
                hull = lamps.len();
            }
        }
        if lamps.len() > MAX_SHIP_LIGHTS {
            return Err(format!("{} lights, more than the buffer's {MAX_SHIP_LIGHTS}", lamps.len()));
        }
        let mut list = LightList { lamps, hull };
        list.place(rig, values).map_err(|e| e.to_string())?;
        Ok(list)
    }

    /// Each slot's place with the rigs' values `values`.
    pub fn place(&mut self, rig: &ShipRig, values: &se_core::elite_rig::Values) -> Result<(), se_core::elite_rig::RigError> {
        let bones: Vec<String> = self.lamps.iter().map(|l| l.bone.clone()).collect();
        for (lamp, place) in self.lamps.iter_mut().zip(rig.bone_frames(values, &bones)?) {
            lamp.place = place;
        }
        Ok(())
    }

    /// The range of the buffer a material lights with (A6): the outside's the hull's lights, the seat's the flagged ones (one
    /// run by `new`'s order).
    pub fn range(&self, inside: bool) -> UVec4 {
        if !inside {
            return UVec4::new(0, self.hull as u32, 0, 0);
        }
        let first = self.lamps.iter().position(Lamp::flagged).unwrap_or(0);
        let count = self.lamps[first..].iter().take_while(|l| l.flagged()).count();
        UVec4::new(first as u32, count as u32, 0, 0)
    }
}

// ---- The parameters ----

/// The parameters' values this frame (the module's notes say what feeds each).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Params {
    pub headlights: bool,
    pub gear_deployed: bool,
    pub overheating: bool,
    pub active_dissipation: f32,
    pub passive_dissipation: f32,
}

impl Params {
    /// From the ship's heat fraction `f`, whether its radiator runs, the pilot's lights switch and the gear.
    pub fn of(f: f64, radiator: bool, headlights: bool, gear_deployed: bool) -> Params {
        let (f0, f1) = (elite_sim::heat::DISSIPATE_F0, elite_sim::heat::DISSIPATE_F1);
        let on = |x: f64| if radiator { x.clamp(0.0, 1.0) as f32 } else { 0.0 };
        Params { headlights, gear_deployed, overheating: f >= OVERHEAT_WARNING, active_dissipation: on((f - f0) / (1.0 - f0)), passive_dissipation: on(f / f1) }
    }

    /// A bool parameter by name; one the game doesn't feed is false (a generic parameter is made with 0).
    pub fn bool(&self, name: &str) -> bool {
        match name {
            "Option_HeadlightsEnabled" => self.headlights,
            "Option_LandingGearDeployed" => self.gear_deployed,
            "Warning_Overheating" => self.overheating,
            "Warning_DangerProximity" => DANGER_PROXIMITY,
            _ => false,
        }
    }

    /// A float parameter by name; one the mod doesn't know is 1, which leaves the level as it is.
    pub fn float(&self, name: &str) -> f32 {
        match name {
            "HeatRadiator_VisualActiveDissipation" => self.active_dissipation,
            "HeatRadiator_VisualPassiveDissipation" => self.passive_dissipation,
            "cockpitAmberLightIntensity" => AMBER,
            "OverrideBrightnessScale" => OVERRIDE_BRIGHTNESS,
            _ => 1.0,
        }
    }
}

// ---- The heat effect (ShipHeatEffectActivity) ----

/// The heat effect's numbers (OperationsMode's states' `ShipHeatEffectActivity`, re/elite/work/csm/vfx/notes.md).
#[derive(Clone, Debug, PartialEq)]
pub struct HeatEffect {
    pub element: String,
    pub emissive: [Vec3; 2],
    pub map_scales: [Vec3; 2],
    pub up_s: f32,
    pub down_s: f32,
    pub exponent: f32,
}

impl HeatEffect {
    /// The effect a state starts, if it has one.
    pub fn of_state(s: &se_core::elite_ship::State) -> Option<HeatEffect> {
        s.activities.iter().find_map(|a| match a {
            Activity::Other(o) if o.kind == "ShipHeatEffectActivity" => {
                let g = o.fields.get("glow")?;
                let v3 = |k: &str| -> Option<Vec3> { Some(Vec3::from_array(<[f32; 3]>::try_from(g.get(k)?.as_array()?.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect::<Vec<_>>()).ok()?)) };
                let f = |k: &str| g.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
                Some(HeatEffect {
                    element: g.get("material_element")?.as_str()?.to_string(),
                    emissive: [v3("emissive_cold")?, v3("emissive_hot")?],
                    map_scales: [v3("map_scales_cold")?, v3("map_scales_hot")?],
                    up_s: f("heat_up_time")?,
                    down_s: f("cool_down_time")?,
                    exponent: f("glow_exponent")?,
                })
            }
            _ => None,
        })
    }

    /// The level moved towards `target` over `dt` (job 0x142de85b0: rising over heat_up_time, falling over cool_down_time,
    /// each the seconds for 0..1).
    pub fn follow(&self, level: f32, target: f32, dt: f32) -> f32 {
        if target > level {
            (level + if self.up_s > 0.0 { dt / self.up_s } else { 1.0 }).min(target)
        } else {
            (level - if self.down_s > 0.0 { dt / self.down_s } else { 1.0 }).max(target)
        }
    }

    /// The element's attributes at `level` (0x1438149f0: glow = level ^ exponent lerps cold to hot); the emissive in this
    /// game's units is the shader's (`self_illum_scale` carries U).
    pub fn glow(&self, level: f32) -> HeatGlow {
        let g = level.max(0.0).powf(self.exponent);
        HeatGlow { emissive: self.emissive[0].lerp(self.emissive[1], g).extend(0.0), map_scales: self.map_scales[0].lerp(self.map_scales[1], g).extend(0.0) }
    }
}

// ---- Running them ----

/// Each slot's animator and draws, the heat effect's level, what was last written, a test's overrides.
#[derive(Resource, Default)]
pub struct LightsState {
    animators: Vec<Animator>,
    rngs: Vec<Rng>,
    levels: Vec<f32>,
    heat_level: f32,
    written: Vec<ShipLight>,
    /// The scene's inputs as last written, and whether the adapted compensation applied.
    scene: Option<(SceneExposure, bool)>,
    glow: Option<HeatGlow>,
    placed: u64,
    /// `sw lights heat F|auto`, `sw lights head on|off|auto`: a test's heat fraction and lights switch over the pilot's.
    pub dev_heat: Option<f64>,
    pub dev_headlights: Option<bool>,
    pub log: bool,
}

/// One frame: the parameters, every slot's level, the buffers written where they changed.
#[allow(clippy::too_many_arguments)]
pub fn update(
    assets: Res<SidewinderAssets>,
    parts: Res<Parts>,
    pilot: Res<Pilot>,
    time: Res<Time>,
    list: Option<ResMut<LightList>>,
    lights: Option<ResMut<ShipLights>>,
    scene: Option<Res<ShipScene>>,
    glow: Option<Res<ShipGlow>>,
    mut state: ResMut<LightsState>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    cams: Query<&ColorGrading, With<crate::camera::MainCamera>>,
    auto: Res<crate::exposure::AutoExposureSettings>,
    photo: Res<crate::exposure::PhotoMode>,
) {
    let (Some(mut list), Some(mut lights), Some(scene), Some(glow)) = (list, lights, scene, glow) else { return };
    let states = parts.states();
    if states.is_empty() {
        return;
    }
    let rig = &assets.rig;
    if state.placed != parts.revision() {
        if let Err(e) = list.place(rig, parts.values()) {
            warn!("sidewinder: the lights can't be placed: {e}");
        }
        state.placed = parts.revision();
    }
    let n = list.lamps.len();
    if state.animators.len() != n {
        state.rngs = (0..n as u32).map(|i| Rng::new(i + 1)).collect();
        let st = &mut *state;
        st.animators = list.lamps.iter().zip(st.rngs.iter_mut()).map(|(l, r)| Animator::new(&l.animator, r)).collect();
        st.levels = vec![0.0; n];
    }
    // The scene's exposure inputs for the camera's exposure, whether the adapted compensation applies, the slots' terms.
    let exposure = cams.iter().next().map_or(0.0, |g| g.global.exposure);
    let now = (scene_inputs(exposure, auto.target_bright), photo.auto());
    if state.scene != Some(now) {
        let terms: Vec<Vec4> = list.lamps.iter().map(Lamp::exposure_terms).collect();
        scene.write(&mut buffers, &now.0, now.1, &terms);
        state.scene = Some(now);
    }
    // The parameters.
    let f = state.dev_heat.unwrap_or_else(|| pilot.heat.fraction(&pilot.heat_spec));
    let p = Params::of(f, pilot.powered && !pilot.silent, state.dev_headlights.unwrap_or(pilot.lights), parts.gear_down().unwrap_or(true));
    let dt = time.delta_secs();
    let st = &mut *state;
    let mut records = Vec::with_capacity(n);
    for (i, lamp) in list.lamps.iter().enumerate() {
        // Out of its state the activity is stopped: no light, and nothing steps its animator (the module's notes).
        if !states.get(lamp.machine).is_some_and(|s| *s == lamp.state) {
            st.levels[i] = 0.0;
            records.push(lamp.record(0.0));
            continue;
        }
        let a = &lamp.animator;
        let bools: Vec<bool> = a.bool_params.iter().map(|b| p.bool(b)).collect();
        let floats: Vec<f32> = a.float_params.iter().map(|x| p.float(x)).collect();
        // (The exposure factor, which multiplies the whole level, is the shader's: the level here is without it.)
        let level = st.animators[i].step(a, &lamp.steps, dt, true, &bools, &floats, (lamp.light.intensity as f32, lamp.light.float_120 as f32), 1.0, &mut st.rngs[i]);
        st.levels[i] = level;
        records.push(lamp.record(level));
    }
    if records != st.written {
        lights.write(&mut buffers, &records);
        st.written = records;
    }
    // The heat effect's glow, from OperationsMode's state.
    let effect = rig.ship.machines.iter().enumerate().filter(|(i, m)| m.name == "OperationsMode" && rig.machine_owner(*i) == MachineOwner::Hull).find_map(|(i, m)| HeatEffect::of_state(m.state(states.get(i)?)?));
    if let Some(e) = effect {
        st.heat_level = e.follow(st.heat_level, p.passive_dissipation, dt);
        let g = e.glow(st.heat_level);
        if st.glow != Some(g) {
            glow.write(&mut buffers, &g);
            st.glow = Some(g);
        }
    }
    if std::mem::take(&mut st.log) {
        log_lights(&list, st, states, rig, &p);
    }
}

/// `sw lights`: every light (its level without the exposure factor, and the factor at the camera's own exposure: in photo
/// mode 2 the shader's is higher by the adapted compensation), the scene's inputs and the glow.
fn log_lights(list: &LightList, st: &LightsState, states: &[String], rig: &ShipRig, p: &Params) {
    let scene_ev = st.scene.map_or(0.0, |(s, _)| s.scene_ev);
    for (i, lamp) in list.lamps.iter().enumerate() {
        let r = lamp.record(st.levels[i]);
        let machine = rig.ship.machines.get(lamp.machine).map_or("?", |m| m.name.as_str());
        let now = states.get(lamp.machine).map_or("?", String::as_str);
        info!(
            "sidewinder light {i:2} {}{}: {} '{}' at {} ({machine} {} {}, now {now}); level {:.4} x factor {:.3}, colour {:.4} {:.4} {:.4}, range {:.2} m, animator {}",
            if i < list.hull { "hull" } else { "cockpit" },
            if lamp.flagged() { " (seat)" } else { "" },
            if lamp.spot { "spot" } else { "point" },
            lamp.holder(),
            lamp.bone,
            if states.get(lamp.machine).is_some_and(|s| *s == lamp.state) { "in" } else { "out of" },
            lamp.state,
            st.levels[i],
            exposure_factor(&lamp.light, scene_ev),
            r.colour.x,
            r.colour.y,
            r.colour.z,
            r.pos_range.w,
            if st.animators[i].moving(&lamp.animator, !lamp.steps.is_empty()) { "running" } else { "still" },
        );
    }
    let (inside, outside) = (list.range(true), list.range(false));
    info!(
        "sidewinder lights: {} slots ({} hull, {} cockpit; the outside's range {}..{}, the seat's {}..{}); parameters {p:?}; scene {:?}; heat glow level {:.3} {:?}",
        list.lamps.len(),
        list.hull,
        list.lamps.len() - list.hull,
        outside.x,
        outside.x + outside.y,
        inside.x,
        inside.x + inside.y,
        st.scene,
        st.heat_level,
        st.glow
    );
}

/// `sw lights [heat F|auto] [head on|off|auto] [state MACHINE STATE|auto] [switch]`: alone it logs every light; the words
/// set a test's heat fraction or headlights switch over the pilot's, a machine's state over the table's, or press the
/// pilot's lights switch (Insert).
pub fn dev_command(world: &mut World, args: &str) {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.as_slice() {
        [] => world.resource_mut::<LightsState>().log = true,
        ["heat", x] => world.resource_mut::<LightsState>().dev_heat = x.parse::<f64>().ok(),
        ["head", w] => world.resource_mut::<LightsState>().dev_headlights = match *w {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        },
        ["state", machine, "auto"] => world.resource_mut::<Parts>().dev_states.retain(|(m, _)| !m.eq_ignore_ascii_case(machine)),
        ["state", machine, rest @ ..] if !rest.is_empty() => {
            let mut parts = world.resource_mut::<Parts>();
            parts.dev_states.retain(|(m, _)| !m.eq_ignore_ascii_case(machine));
            parts.dev_states.push((machine.to_string(), rest.join(" ")));
        }
        ["switch"] => world.resource_mut::<Pilot>().dev_switch = Some(super::pilot::Switch::Lights),
        _ => warn!("sw lights [heat F|auto] [head on|off|auto] [state MACHINE STATE|auto] [switch]"),
    }
}

pub fn build(app: &mut App) {
    app.init_resource::<LightsState>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anim(f: impl FnOnce(&mut LightAnimator)) -> LightAnimator {
        let mut a = LightAnimator {
            hard: false,
            jitter_spread: 0,
            jitter_below: 0.0,
            jitter_above: 0.0,
            jitter_rate_hz: 0.0,
            jitter_chance: Some(1.0),
            blink_rate: 0.0,
            blink_start: 0.0,
            blink_extra: 0.0,
            blink_duty: 0.5,
            pattern: None,
            pattern_cycle_s: 1.0,
            hold_s: 0.0,
            fade_in_s: 0.0,
            fade_out_s: 0.0,
            exponent: 1.0,
            float_params: vec![],
            bool_params: vec![],
            bool_mode: 0,
            colour_param: None,
        };
        f(&mut a);
        a
    }

    /// Runs an animator `secs` at 1/240 s steps, giving its level at each step's end.
    fn run(a: &LightAnimator, steps: &[bool], secs: f32, on: bool, bools: &[bool], floats: &[f32]) -> Vec<f32> {
        let mut rng = Rng::new(7);
        let mut s = Animator::new(a, &mut rng);
        (0..(secs * 240.0).round() as usize).map(|_| s.step(a, steps, 1.0 / 240.0, on, bools, floats, (400.0, 25.0), 1.0, &mut rng)).collect()
    }

    /// The red beacon's pattern (f25 n1 f25 over 2 s): on for one 2/51 s step from 25/51 x 2 s, every 2 s; off
    /// otherwise; no fades, so on means the full level.
    #[test]
    fn a_pattern_blinks_on_its_steps() {
        let a = anim(|a| {
            a.pattern = Some("f25n1f25".into());
            a.pattern_cycle_s = 2.0;
        });
        let steps = parse_pattern("f25n1f25").unwrap();
        let levels = run(&a, &steps, 4.0, true, &[], &[]);
        let on: Vec<f32> = levels.iter().enumerate().filter(|(_, l)| **l > 25.0).map(|(i, _)| i as f32 / 240.0).collect();
        let step = 2.0 / 51.0;
        assert!(!on.is_empty());
        for t in &on {
            let phase = t % 2.0;
            assert!(phase >= 25.0 * step - 0.01 && phase <= 26.0 * step + 0.01, "on at {t}");
        }
        assert!(on.iter().any(|t| *t < 2.0) && on.iter().any(|t| *t > 2.0), "every cycle");
        assert!((on.len() as f32 / 240.0 - 2.0 * step).abs() < 0.02, "one step a cycle: {} s", on.len() as f32 / 240.0);
        assert!(levels.iter().all(|&l| l == 400.0 || l == 25.0), "the on level or the off level");
    }

    /// The timed blink (the red warning lamp's rate 3, duty 0.5): on and off phases of 1/3 s each, from off.
    #[test]
    fn the_timed_blink() {
        let a = anim(|a| a.blink_rate = 3.0);
        let levels = run(&a, &[], 2.0, true, &[], &[]);
        let switches = levels.windows(2).filter(|w| (w[0] > 25.0) != (w[1] > 25.0)).count();
        assert_eq!(levels[10], 25.0, "starts off");
        assert!((5..=6).contains(&switches), "{switches} switches in 2 s of 1/3 s phases");
    }

    /// Fades and hold: a 0.1 s fade in, a 1 s hold then a 0.4 s fade out after the switch goes off; the bools switch it
    /// (mode 0: all), the floats' product scales it, raised to the exponent.
    #[test]
    fn fades_hold_and_parameters() {
        let a = anim(|a| {
            a.fade_in_s = 0.1;
            a.fade_out_s = 0.4;
            a.hold_s = 1.0;
        });
        let mut rng = Rng::new(1);
        let mut s = Animator::new(&a, &mut rng);
        let mut step = |on: bool, secs: f32, s: &mut Animator| (0..(secs * 240.0) as usize).map(|_| s.step(&a, &[], 1.0 / 240.0, on, &[true], &[1.0], (400.0, 0.0), 1.0, &mut rng)).last().unwrap();
        // (The parameters' product starts at 0 and is limited by the fade in too, so the first rise is fade x product.)
        assert!((step(true, 0.05, &mut s) - 100.0).abs() < 4.0, "a quarter in at 0.05 s");
        assert!((step(true, 0.1, &mut s) - 400.0).abs() < 1e-3);
        assert!((step(false, 0.9, &mut s) - 400.0).abs() < 1e-3, "held");
        assert!((step(false, 0.3, &mut s) - 200.0).abs() < 10.0, "halfway out 0.2 s after the hold");
        assert_eq!(step(false, 0.3, &mut s), 0.0);
        // A bool off switches it off; the floats' product to the exponent scales it.
        let b = anim(|a| a.exponent = 8.0);
        let mut rng = Rng::new(2);
        let mut s = Animator::new(&b, &mut rng);
        assert_eq!(s.step(&b, &[], 0.1, true, &[true, false], &[], (400.0, 0.0), 1.0, &mut rng), 0.0, "all must be true");
        let x = s.step(&b, &[], 0.1, true, &[true, true], &[0.5, 1.0], (250.0, 0.0), 1.0, &mut rng);
        assert!((x - 250.0 * 0.5f32.powi(8)).abs() < 1e-3, "{x}");
        // Out of its state the light goes off (no hold, no fade: at once).
        assert_eq!(s.step(&b, &[], 0.1, false, &[true, true], &[0.5], (250.0, 0.0), 1.0, &mut rng), 0.0);
    }

    /// The jitter: drawn in [-below, above] every period, faded with the level unless hard; the vents' rate 60 Hz.
    #[test]
    fn jitter() {
        let a = anim(|a| {
            a.jitter_below = 50.0;
            a.jitter_above = 50.0;
            a.jitter_rate_hz = 60.0;
        });
        let levels = run(&a, &[], 1.0, true, &[], &[1.0]);
        assert!(levels.iter().all(|&l| (350.0..=450.0).contains(&l)), "400 +- 50");
        // (A redraw once the timer is past the period, strictly: at 1/240 s steps every fifth step, 48 a second.)
        let distinct = levels.windows(2).filter(|w| w[0] != w[1]).count();
        assert!((46..=48).contains(&distinct), "{distinct} redraws in a second");
        // Hard: the jitter isn't faded, so a light switched off still flickers by it about its off level (the fuel lights'
        // [-5, 0]; here off 25).
        let h = anim(|a| {
            a.hard = true;
            a.jitter_below = 5.0;
            a.jitter_rate_hz = 15.0;
        });
        let off = run(&h, &[], 1.0, true, &[false], &[]);
        assert!(off.iter().all(|&l| (20.0..=25.0).contains(&l)) && off.iter().any(|&l| l < 24.9), "25 + [-5, 0]");
    }

    /// The exposure factor: 1 with exposure off; else 2^(clamp(scene + bias, min, max) - 3) (the cockpit lamps' 8..8.5).
    #[test]
    fn exposure_factor_and_its_clamps() {
        let mut l: Light = serde_json::from_value(serde_json::json!({
            "bones": [], "holder": null, "bytes_88": [0, 0, 0, 0], "colour": [1.0, 1.0, 1.0], "exposure_enable": true, "exposure_bias": 0.0,
            "floats_112": [8.0, 8.5], "float_120": 0.0, "intensity": 1.0, "range_m": 1.0, "source_size": 0.1
        }))
        .unwrap();
        assert_eq!(exposure_factor(&l, 0.0), 32.0);
        assert_eq!(exposure_factor(&l, 20.0), (8.5f32 - 3.0).exp2());
        assert!((exposure_factor(&l, 8.25) - 5.25f32.exp2()).abs() < 1e-4);
        l.exposure_bias = -1.0;
        assert!((exposure_factor(&l, 9.25) - 5.25f32.exp2()).abs() < 1e-4);
        l.exposure_enable = false;
        assert_eq!(exposure_factor(&l, 9.25), 1.0);
        // The scene's inputs: a stop more exposure is a stop less scene; the rescaling's two values follow it.
        let (a, b) = (scene_inputs(0.0, 0.35), scene_inputs(1.0, 0.35));
        assert!((a.scene_ev - b.scene_ev - 1.0).abs() < 1e-5);
        assert!((a.luminance - a.scene_ev.exp2()).abs() < 1e-3 * a.luminance && (a.emissive_multiplier * 8.0 - a.luminance).abs() < 1e-3 * a.luminance);
        assert_eq!(a.self_illum_scale, ELITE_UNIT * SELF_ILLUM);
    }

    /// The parameters: the dissipations by FUN_144399d90's rule, 0 with the radiator off; the warning at 1.5.
    #[test]
    fn parameters() {
        let p = Params::of(0.4, true, false, true);
        assert!((p.active_dissipation - 0.4).abs() < 1e-6 && (p.passive_dissipation - 0.4).abs() < 1e-6 && !p.overheating);
        assert_eq!(Params::of(2.0, true, true, false).active_dissipation, 1.0);
        assert!(Params::of(1.5, true, true, false).overheating);
        assert_eq!(Params::of(0.9, false, true, false).passive_dissipation, 0.0);
        assert!(p.bool("Option_LandingGearDeployed") && !p.bool("Option_HeadlightsEnabled") && !p.bool("Warning_DangerProximity"));
        assert_eq!((p.float("cockpitAmberLightIntensity"), p.float("NoSuchParameter")), (1.0, 1.0));
    }

    /// The list on the mod's ship: the hull's lights first (its unflagged, then the vents'), then the cockpit's (its flagged,
    /// then the red warning lamp's two); each material's range; every slot placed; the headlights' spots point along the nose
    /// (-Z) from in front of the cockpit.
    #[test]
    fn the_list_and_its_ranges() {
        let read = |rel: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(rel)).map_err(|e| e.to_string());
        let rig = super::super::assets::load(&read).unwrap_or_else(|(f, why)| panic!("{f}: {why}")).assets.rig;
        let list = LightList::new(&rig, &rig.static_pose(true)).unwrap();
        assert_eq!((list.hull, list.lamps.len()), (47, 84), "the hull's 47 slots, the cockpit's 37");
        assert!(list.lamps[..list.hull].iter().all(|l| rig.machine_owner(l.machine) == MachineOwner::Hull && l.owner == MachineOwner::Hull));
        assert!(list.lamps[list.hull..].iter().all(|l| rig.machine_owner(l.machine) == MachineOwner::Cockpit && l.owner == MachineOwner::Cockpit));
        assert_eq!(list.range(false), UVec4::new(0, 47, 0, 0));
        assert_eq!(list.range(true), UVec4::new(27, 55, 0, 0), "the 20 vent lights and the cockpit's 35 flagged lamps");
        let seat = &list.lamps[27..82];
        assert!(seat.iter().all(Lamp::flagged) && list.lamps[..27].iter().chain(&list.lamps[82..]).all(|l| !l.flagged()));
        assert!(seat[..20].iter().all(|l| rig.ship.machines[l.machine].name == "HeatVents"));
        assert!(list.lamps[82..].iter().all(|l| l.holder() == "Light_Warning_Red"));
        // The exposure terms: the cockpit's lamps 8..8.5 or 8.5..9 with no bias, the hull's off.
        assert!(list.lamps[..list.hull].iter().all(|l| l.exposure_terms() == Vec4::ZERO));
        assert!(list.lamps[list.hull..].iter().all(|l| { let t = l.exposure_terms(); t.x == 1.0 && t.y == 0.0 && (t.z == 8.0 || t.z == 8.5) && t.w == t.z + 0.5 }));
        assert!(list.lamps.iter().all(|l| l.place.is_some()), "{:?}", list.lamps.iter().filter(|l| l.place.is_none()).map(|l| &l.bone).collect::<Vec<_>>());
        let head = list.lamps.iter().find(|l| l.spot && l.light.ies_profile.is_some()).unwrap();
        let (pos, axis) = head.place.unwrap();
        assert!(axis.z < -0.99 && pos.z < -5.0, "{pos} {axis}");
        let r = head.record(1.0);
        assert!((r.dir_cos.w - 30f32.to_radians().cos()).abs() < 1e-5 && (r.spot.x - 27f32.to_radians().cos()).abs() < 1e-5 && (r.spot.y - 3.0).abs() < 1e-4 && r.spot.z == 1.0);
        // The heat effect of the parked ship's OperationsMode state, and its glow's ends.
        let ops = rig.ship.machines.iter().find(|m| m.name == "OperationsMode").unwrap();
        let e = HeatEffect::of_state(ops.state("Docked").unwrap()).unwrap();
        assert_eq!((e.element.as_str(), e.emissive[1], e.exponent), ("ElementGlow", Vec3::new(1750.0, 120.0, 20.0), 3.0));
        assert_eq!(e.glow(0.0), HeatGlow::default());
        assert_eq!(e.glow(1.0).map_scales, Vec4::new(1.0, 0.1, 0.5, 0.0));
        assert!((e.follow(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6 && e.follow(0.9, 0.0, 2.0) == 0.0);
    }
}
