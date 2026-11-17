//! When Elite posts each of the ship's sound events and how it sets each game parameter, state and switch: the rules of
//! `re/elite/frea_export/sidewinder_audio/triggers.json` (its entry ids name each rule below), as pure code. The ship's
//! state goes in as a [`ShipView`] once a frame; what Elite's audio objects would call comes out as [`Call`]s, which
//! `mod.rs` hands to the mixer. Nothing here reads Bevy or the mixer, so the rules are tested alone.
//!
//! Only entries whose certainty is `proven`, `proven, one link inferred` or `measured` are here. The inferred link is
//! nearly always the same (the notes' "feeders": what fills the audio component's fields was not found), so a field's
//! meaning is the event it posts, and the motion inputs' units are the bank curves' x ranges (marked `UNIT INFERRED`).
//! What the ship's state cannot drive (no such state in the flight) is listed in `mod.rs`.

use elite_sim::drive::Drive;

/// Something Elite's audio objects call: post an event on an emitter, set a global state, set a game parameter (global:
/// Elite's per-object values are not known per emitter, and the mixer falls back to the global one).
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Post(String, Emitter),
    State(&'static str, &'static str),
    Gp(&'static str, f32),
}

/// The game objects events are posted on. Their names are the state machines' (`cockpit.json`'s loop tables) and the
/// notes' (`Internal_<loop>` for the internal reporter, `Internal_Cockpit_Origin` for silent running); `Voice` is ours:
/// the data keys nothing on an emitter's name, only begin and end must meet on the same one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Emitter {
    /// The internal reporter's drive loop of this index in `DRIVE_LOOPS`.
    Drive(u8),
    /// A cockpit reporter position (index in `POSITIONS`).
    Cockpit(u8),
    /// `Internal_Cockpit_Origin`.
    CockpitOrigin,
    /// The ship's own object, `Internal_ShipOrigin`, the ShipOrigin drive loop's (its id is that loop's): boost,
    /// flight assist, the drive, modes, shields. (The boost events set the BOOST switch on the object they are posted on,
    /// and the ShipOrigin loop's boost layer reads it there: so they must share it.)
    Ship,
    /// The ship voice's lines.
    Voice,
    /// A cue at a set of bones (`ShipSound::post_at`): the event's id, so each cue keeps its own position.
    Cue(u32),
}

impl Emitter {
    /// The mixer's id of the emitter.
    pub fn id(self) -> u32 {
        match self {
            Emitter::Drive(i) => 0x100 + u32::from(i),
            Emitter::Cockpit(i) => 0x200 + u32::from(i),
            Emitter::CockpitOrigin => 0x300,
            Emitter::Ship => Emitter::Drive(0).id(),
            Emitter::Voice => 0x302,
            Emitter::Cue(id) => id,
        }
    }
}

/// `engine_loops_start`: the internal reporter's 8 drive loops.
pub const DRIVE_LOOPS: [&str; 8] = ["ShipOrigin", "MainDriveL", "MainDriveR", "MainDriveC", "RetroDriveL", "RetroDriveR", "RetroDriveC", "Front_Centre"];
/// The cockpit reporter's positions in `cockpit.json`'s order; `Mid_Centre` is on `Internal_Cockpit_Origin`.
pub const POSITIONS: [&str; 9] = ["Back_Centre", "Back_Left", "Back_Right", "Front_Centre", "Front_Left", "Front_Right", "Mid_Centre", "Mid_Left", "Mid_Right"];
/// The engines-off delay before the drive loops stop (`engines_on_off`: `+0x918 = 3.0`).
pub const ENGINES_OFF_S: f64 = 3.0;
/// `carried_02`: the heat voice's thresholds on the heat ratio and its cooldown.
pub const HEAT_CRITICAL: f64 = 1.2;
pub const HEAT_DAMAGE: f64 = 1.5;
pub const HEAT_REPORT_S: f64 = 30.0;
/// `carried_05`: the hull share under which "hull integrity critical" is said once.
pub const HULL_CRITICAL: f64 = 0.25;
/// `motion_rtpcs`: the slew rates (the csm's motion_rtpcs, all 3.0 for the Sidewinder), speed_relative's 5.0 above a 5.0
/// gap, speed_rot_smooth's 0.665, and speed_relative's clamp.
pub const SLEW: f64 = 3.0;
pub const SLEW_RELATIVE: f64 = 5.0;
pub const SLEW_ROT: f64 = 0.665;
pub const RELATIVE_MOST: f64 = 199.9;
/// `player_inside_switch`: seated in the own cockpit. 1 is inside, proven from the banks (T5c,
/// `re/elite/work/sound/t5c/inside2.py`): the Init bank's switch group on this parameter (062c4b2d) selects d547ba4f
/// below 0.5 and f6dde0fa from 0.5; under d547ba4f sit 391 recordings of the `*_ext_data` banks (the exterior engines,
/// e.g. `engine_main_ext_01`), while under f6dde0fa sits no recording of an exterior bank alone and 11 of the containers
/// (the external loops' among them) play nothing there (the ship's outside is not heard from inside). So 0 is outside and 1
/// inside; the shields' recordings labelled `_int` / `_ext` by the old export's roles are the other way round.
pub const INSIDE: f32 = 1.0;

/// The audio component's motion floats (`motion_rtpcs`: offsets +0x00..+0x44), as the feeder (`motion_in`) fills them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionIn {
    /// +0x00 `spaceship_accel_self`.
    pub accel: f64,
    /// +0x04 (`spaceship_speed_self`, and speed_relative with flight assist on).
    pub speed: f64,
    /// +0x08 (speed_relative with flight assist off).
    pub speed_fa_off: f64,
    /// +0x0c `spaceship_speed_physical_self`.
    pub speed_physical: f64,
    /// +0x10, +0x14 (no curve at rest uses them).
    pub accel_normalised: f64,
    pub accel_dir_normalised: f64,
    /// +0x18 (smoothed into `_relative_smooth` and `_relative_slow`).
    pub accel_relative: f64,
    /// +0x1c (smoothed into `spaceship_accel_directional_self`).
    pub accel_directional: f64,
    /// +0x20 (smoothed into `spaceship_target_speed_relative_self`).
    pub target_speed: f64,
    /// +0x34, +0x38, +0x3c, +0x40, +0x44.
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
    pub speed_rot: f64,
    pub out_of_control_yaw: f64,
}

/// What the feeder reads from the flight (ship axes: X right, Y up, -Z the nose).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flight {
    /// Velocity and acceleration in ship axes (m/s, m/s^2).
    pub vel: [f64; 3],
    pub acc: [f64; 3],
    /// Spin in ship axes (rad/s: x pitch nose up, y yaw nose left, z roll).
    pub omega: [f64; 3],
    /// This block's top speed and forward acceleration (m/s, m/s^2), the reverse speed's share of the top.
    pub top_speed: f64,
    pub accel_fwd: f64,
    pub back_share: f64,
    pub throttle: f64,
}

/// The feeder (the notes' open item: the code that fills the component was not found). Each field's reading is from
/// its curves' x range and the notes; every one is `UNIT INFERRED` except where said.
pub fn motion_in(f: &Flight) -> MotionIn {
    let fwd_speed = -f.vel[2];
    let speed = (f.vel[0] * f.vel[0] + f.vel[1] * f.vel[1] + f.vel[2] * f.vel[2]).sqrt();
    let acc_len = (f.acc[0] * f.acc[0] + f.acc[1] * f.acc[1] + f.acc[2] * f.acc[2]).sqrt();
    let fwd_acc = -f.acc[2];
    let top = f.top_speed.max(1e-6);
    let amax = f.accel_fwd.max(1e-6);
    let w = (f.omega[0] * f.omega[0] + f.omega[1] * f.omega[1] + f.omega[2] * f.omega[2]).sqrt();
    MotionIn {
        // UNIT INFERRED: m/s^2 along the nose (curves -100..100).
        accel: fwd_acc,
        // UNIT INFERRED: the speed along the nose in percent of this block's top speed (curves 0..150: boost reaches 145 %;
        // speed_relative's -200..200 "percent" reads the same field with flight assist on).
        speed: fwd_speed / top * 100.0,
        // UNIT INFERRED: with flight assist off the exe reads another field; here the whole speed in percent, signed by the
        // nose's component (the field's meaning is not known).
        speed_fa_off: speed.copysign(fwd_speed) / top * 100.0,
        // m/s by the range (the notes' reading: curves 0..1000).
        speed_physical: speed,
        // (No curve at rest reads these two.) The acceleration's share of the forward one, along the nose and whole.
        accel_normalised: fwd_acc / amax,
        accel_dir_normalised: acc_len / amax,
        // UNIT INFERRED: the acceleration along the nose in percent of the forward thrust's (curves -100..100).
        accel_relative: fwd_acc / amax * 100.0,
        // UNIT INFERRED: the whole acceleration in percent of the forward thrust's (curves -100..100).
        accel_directional: acc_len / amax * 100.0,
        // UNIT INFERRED: the throttle's target speed in percent of the top (curves -150..150; reverse is the back share).
        target_speed: if f.throttle >= 0.0 { f.throttle * 100.0 } else { f.throttle * f.back_share * 100.0 },
        // UNIT INFERRED: rad/s for pitch and roll (curves -2..2), degrees a second for yaw (curves -30..30).
        pitch: f.omega[0],
        yaw: f.omega[1].to_degrees(),
        roll: f.omega[2],
        // UNIT INFERRED: the spin's rate in rad/s (curves 0..1, smoothed 0..2).
        speed_rot: w,
        // The flight is never out of control here.
        out_of_control_yaw: 0.0,
    }
}

/// The ship's state this frame, as the triggers read it.
#[derive(Clone, Copy, Debug)]
pub struct ShipView {
    /// Real seconds since the last frame.
    pub dt: f64,
    /// 1 normal space, 2 supercruise, 3 hyperspace (`audio_mode_transitions` proves the numbers).
    pub mode: u8,
    /// The power plant runs (the engines-on flag).
    pub engines_on: bool,
    /// 0 idle, 1 warm-up, 2 firing, 3 cool-down.
    pub boost: u8,
    /// The boost key was pressed this frame and refused (nothing started).
    pub boost_refused: bool,
    pub flight_assist: bool,
    pub gear_down: bool,
    pub scoop: bool,
    pub lights: bool,
    pub silent: bool,
    pub canopy_breached: bool,
    /// The emergency oxygen runs (the pilot's oxygen mask: `elite_sim::life::Life::running`).
    pub oxygen_mask: bool,
    /// The heat ratio (1.0 = 100 %).
    pub heat: f64,
    /// The hull's share left.
    pub hull: f64,
    pub shields_up: bool,
    /// The shield generator has power (off by silent running, the module switch or no fuel).
    pub shields_powered: bool,
    pub drive: Drive,
    /// The drive's key was pressed this frame.
    pub jump_pressed: bool,
    pub safe_drop: Option<bool>,
    pub glide: bool,
    pub motion: MotionIn,
    /// The pips (0..4) of systems, engines, weapons.
    pub pips: [f64; 3],
}

#[cfg(test)]
impl ShipView {
    /// A ship at rest in normal space, everything working, for the sound tests.
    pub fn at_rest() -> ShipView {
        ShipView {
            dt: 1.0 / 60.0,
            mode: 1,
            engines_on: true,
            boost: 0,
            boost_refused: false,
            flight_assist: true,
            gear_down: true,
            scoop: false,
            lights: false,
            silent: false,
            canopy_breached: false,
            oxygen_mask: false,
            heat: 0.2,
            hull: 1.0,
            shields_up: true,
            shields_powered: true,
            drive: Drive::Ready,
            jump_pressed: false,
            safe_drop: None,
            glide: false,
            motion: MotionIn::default(),
            pips: [2.0; 3],
        }
    }
}

/// The heat voice (`carried_02`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HeatVoice {
    cooldown: f64,
    /// Above 1.5: whether the next line is "taking heat damage" (None: not alternating).
    next_damage: Option<bool>,
}

impl HeatVoice {
    pub fn step(&mut self, h: f64, dt: f64) -> Option<&'static str> {
        const CRITICAL: &str = "VoxV01_WarningTemperatureCritical";
        const DAMAGE: &str = "VoxV01_WarningTakingHeatDamage";
        self.cooldown -= dt;
        if h >= HEAT_DAMAGE {
            let say = match self.next_damage {
                None => Some(true),
                Some(d) if self.cooldown <= 0.0 => Some(d),
                Some(_) => None,
            }?;
            self.next_damage = Some(!say);
            self.cooldown = HEAT_REPORT_S;
            return Some(if say { DAMAGE } else { CRITICAL });
        }
        self.next_damage = None;
        if h > HEAT_CRITICAL && self.cooldown <= 0.0 {
            self.cooldown = HEAT_REPORT_S;
            return Some(CRITICAL);
        }
        None
    }
}

/// The hull voice's three once-flags (`carried_05`): cleared only with a new ship.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HullVoice {
    compromised: bool,
    taking_damage: bool,
    critical: bool,
    last: Option<f64>,
}

impl HullVoice {
    pub fn step(&mut self, hull: f64, out: &mut Vec<&'static str>) {
        let fell = self.last.is_some_and(|l| hull < l);
        self.last = Some(hull);
        if !fell {
            return;
        }
        if !self.compromised {
            self.compromised = true;
            out.push("VoxV01_HullIntegrityCompromised");
        } else if !self.taking_damage {
            self.taking_damage = true;
            out.push("VoxV01_TakingDamage");
        }
        if hull < HULL_CRITICAL && !self.critical {
            self.critical = true;
            out.push("VoxV01_HullIntegrityCritical");
        }
    }

    /// The audio starts again (boarding): the flags stay, the last hull is taken afresh.
    fn resume(&mut self) {
        self.last = None;
    }
}

/// The voice countdown (`fsd_countdown`): each whole second's drop posts the kind's tick; the voice says 6 -> "five",
/// 4..1 -> "four".."one" then "engage" (5 says nothing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Countdown {
    last: i32,
    lowest_voiced: i32,
}

impl Default for Countdown {
    fn default() -> Self {
        Countdown { last: -1, lowest_voiced: i32::MAX }
    }
}

impl Countdown {
    /// `left`: the countdown's seconds left, `None` while none runs (the exe's sentinel, -1). Returns (tick, voice lines).
    pub fn step(&mut self, left: Option<f64>) -> (bool, &'static [&'static str]) {
        let Some(left) = left else {
            *self = Countdown::default();
            return (false, &[]);
        };
        let n = left.floor() as i32;
        let tick = self.last >= 0 && n < self.last;
        self.last = n;
        let mut voice: &'static [&'static str] = &[];
        if n < self.lowest_voiced {
            self.lowest_voiced = n;
            voice = match n {
                6 => &["VoxV01_Number5"],
                4 => &["VoxV01_Number4"],
                3 => &["VoxV01_Number3"],
                2 => &["VoxV01_Number2"],
                1 => &["VoxV01_Number1", "VoxV01_Engage"],
                _ => &[],
            };
        }
        (tick, voice)
    }
}

/// `motion_rtpcs`' slew: each frame the value moves toward the input by `min(|d|, r sqrt(|d|) dt)`; `r <= 0` snaps.
pub fn slew(v: f64, x: f64, r: f64, dt: f64) -> f64 {
    let d = x - v;
    if r <= 0.0 {
        return x;
    }
    v + d.signum() * d.abs().min(r * d.abs().sqrt() * dt)
}

/// The smoothed motion values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Smooth {
    relative: f64,
    accel: f64,
    directional: f64,
    target: f64,
    rot: f64,
}

/// The charge's kind (`fsd_charge`: 1 supercruise, 2 hyperspace jump), 0 none.
fn charge_kind(d: Drive) -> u8 {
    match d {
        Drive::Charging(_) => 1,
        Drive::JumpCharging(_) | Drive::Departing(_) => 2,
        _ => 0,
    }
}

/// The drive refuses a charge for its cooldown (Arriving, the cooldown, after a jump), and its progress 0..100.
fn cooldown_progress(d: Drive) -> Option<f64> {
    use elite_sim::drive::{stand_in, ARRIVING_AFTER_JUMP_S, ARRIVING_S, COOLDOWN_S};
    let (left, total) = match d {
        Drive::Arriving(t) => (t + COOLDOWN_S, ARRIVING_S + COOLDOWN_S),
        Drive::Cooldown(t) => (t, ARRIVING_S + COOLDOWN_S),
        Drive::Arrived(t) => (t, ARRIVING_AFTER_JUMP_S + stand_in::JUMP_COOLDOWN_S),
        _ => return None,
    };
    Some((100.0 * (1.0 - left / total.max(1e-9))).clamp(0.0, 100.0))
}

/// Every rule's memory between frames. `Triggers::default()` is the audio before it starts.
#[derive(Clone, Debug, Default)]
pub struct Triggers {
    started: bool,
    mode: u8,
    drive_loops: bool,
    engines_on: bool,
    off_timer: f64,
    boost: u8,
    refused_shown: bool,
    flight_assist: bool,
    gear_down: bool,
    scoop: bool,
    lights: bool,
    silent: bool,
    canopy_breached: bool,
    shields_up: bool,
    glide: bool,
    heat: HeatVoice,
    hull: HullVoice,
    drive: Option<Drive>,
    held: bool,
    cooldown: bool,
    countdown: Countdown,
    safe_drop: Option<bool>,
    smooth: Smooth,
    heat_pct: Option<f64>,
    /// The mode transition handle whose `_b` was posted (`audio_mode_transitions`): its `_e` comes with the next one.
    transition: Option<String>,
    /// `cockpit_atmosphere` as last set.
    pressurised: Option<bool>,
}

fn post(out: &mut Vec<Call>, name: impl Into<String>, e: Emitter) {
    out.push(Call::Post(name.into(), e));
}

/// A mode's cockpit loop set (`mode_loops`): its events (without `_b` / `_e`) and emitters.
fn mode_set(mode: u8) -> Vec<(String, Emitter)> {
    let pos = |i: usize| Emitter::Cockpit(i as u8);
    let mut v = Vec::new();
    match mode {
        1 => {
            v.extend(POSITIONS.iter().enumerate().map(|(i, p)| (format!("{p}_#_sidewinder_internal"), pos(i))));
            v.extend(POSITIONS.iter().enumerate().map(|(i, p)| (format!("{p}_SharedCockpit_#"), pos(i))));
        }
        2 => {
            v.extend(POSITIONS.iter().enumerate().map(|(i, p)| (format!("{p}_Supercruise_Cockpit_#"), pos(i))));
            v.extend(POSITIONS.iter().enumerate().map(|(i, p)| (format!("{p}_SharedCockpit_#"), pos(i))));
        }
        _ => v.extend(POSITIONS.iter().enumerate().map(|(i, p)| (format!("{p}_Hyperspace_Cockpit_#"), pos(i)))),
    }
    // (`Mid_Centre` is on the cockpit's origin in every set.)
    for (_, e) in v.iter_mut() {
        if *e == Emitter::Cockpit(6) {
            *e = Emitter::CockpitOrigin;
        }
    }
    v
}

/// orbital_cruise: the supercruise cockpit audio sets state OrbitalCruise On or Off from its status struct (`FUN_144197df0`:
/// +0xa4 when +0xa5 says it changed; the struct's producer is not found). The flight has no orbital cruise: Off whenever
/// supercruise's audio starts (its state OFF takes 96 dB off the layer `0665a352`).
fn orbital_cruise(mode: u8, out: &mut Vec<Call>) {
    if mode == 2 {
        out.push(Call::State("OrbitalCruise", "Off"));
    }
}

fn begin_end(name: &str, begin: bool) -> String {
    name.replace('#', if begin { "b" } else { "e" })
}

fn mode_name(mode: u8) -> &'static str {
    match mode {
        2 => "supercruise",
        3 => "hyperspace",
        _ => "normalflight",
    }
}

fn state_of_mode(mode: u8) -> &'static str {
    match mode {
        2 => "supercruise",
        3 => "hyperspace",
        _ => "normal",
    }
}

impl Triggers {
    /// A new ship: the hull's once-flags go too.
    pub fn new_ship(&mut self) {
        *self = Triggers::default();
    }

    /// The audio stops (getting out, the mod off, the ship lost): everything but the ship's own once-flags, and the points
    /// of no return's ends Elite posts when it tears the audio object down (`fsd_cancel_and_point_of_no_return`).
    pub fn teardown(&mut self, out: &mut Vec<Call>) {
        if self.started {
            post(out, "hyperdrive_PointOfNoReturn_internal_e", Emitter::Ship);
            post(out, "supercruise_PointOfNoReturn_internal_e", Emitter::Ship);
        }
        let mut hull = self.hull;
        hull.resume();
        *self = Triggers { hull, ..Triggers::default() };
    }

    fn drive_loops(&mut self, on: bool, out: &mut Vec<Call>) {
        if self.drive_loops == on {
            return;
        }
        self.drive_loops = on;
        for (i, l) in DRIVE_LOOPS.iter().enumerate() {
            post(out, format!("{l}_{}_sidewinder_internal", if on { "b" } else { "e" }), Emitter::Drive(i as u8));
        }
    }

    fn start(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        self.started = true;
        self.mode = v.mode;
        // engine_loops_start: the loops in mode 1 with the engines on, and state Drives.
        self.engines_on = v.engines_on;
        out.push(Call::State("Drives", if v.engines_on { "On" } else { "Off" }));
        if v.mode == 1 && v.engines_on {
            self.drive_loops(true, out);
        }
        // mode_loops: the mode's cockpit set; frame_shift_state: the state and the mode's enter event (the start is
        // taken as a change from no mode: no "_from_").
        for (n, e) in mode_set(v.mode) {
            post(out, begin_end(&n, true), e);
        }
        out.push(Call::State("FrameShiftState", state_of_mode(v.mode)));
        orbital_cruise(v.mode, out);
        post(out, format!("{}_enter", mode_name(v.mode)), Emitter::Ship);
        // landing_gear: Retracted at set-up, then the gear as it is (no cockpit cue for the start). carried_00: state
        // FlightAssist On when assist is OFF.
        out.push(Call::State("LandingGear", if v.gear_down { "Deployed" } else { "Retracted" }));
        out.push(Call::State("FlightAssist", if v.flight_assist { "Off" } else { "On" }));
        out.push(Call::State("GlideState", if v.glide { "Glide" } else { "NotGlide" }));
        self.cockpit_atmosphere(v, out);
        self.boost = v.boost;
        self.flight_assist = v.flight_assist;
        self.gear_down = v.gear_down;
        self.scoop = v.scoop;
        self.lights = v.lights;
        self.silent = v.silent;
        if v.silent {
            post(out, "SilentRunning_b", Emitter::CockpitOrigin);
        }
        if v.scoop {
            post(out, "CargoScoop_b", Emitter::CockpitOrigin);
        }
        self.canopy_breached = false;
        self.shields_up = v.shields_up;
        self.glide = v.glide;
        self.drive = Some(v.drive);
        self.safe_drop = v.safe_drop;
        self.hull.resume();
        let m = v.motion;
        self.smooth = Smooth { relative: m.speed, accel: m.accel_relative, directional: m.accel_directional, target: m.target_speed, rot: m.speed_rot };
    }

    /// One frame: what Elite would call for this state.
    pub fn step(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        if !self.started {
            self.start(v, out);
        }
        self.modes(v, out);
        self.engines(v, out);
        self.motion(v, out);
        self.boost(v, out);
        self.cockpit(v, out);
        self.cockpit_atmosphere(v, out);
        self.fsd(v, out);
        self.damage(v, out);
    }

    /// audio_mode_transitions, frame_shift_state, mode_loops.
    fn modes(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        if v.mode == self.mode {
            return;
        }
        let (from, to) = (self.mode, v.mode);
        for (n, e) in mode_set(from) {
            post(out, begin_end(&n, false), e);
        }
        // (The internal reporter's drive loops exist only in mode 1.)
        if to != 1 {
            self.drive_loops(false, out);
        } else if self.engines_on {
            self.drive_loops(true, out);
        }
        for (n, e) in mode_set(to) {
            post(out, begin_end(&n, true), e);
        }
        // (`FUN_14427b320` stores the new handle in place of the last, whose release runs its destructor `FUN_1440ac360`:
        // that posts the last one's `_e`, then `FUN_1440cd9a0` posts the new one's `_b`. Nothing else ends it.)
        if matches!((from, to), (1, 2) | (1, 3) | (2, 1) | (2, 3) | (3, 2)) {
            let name = format!("{}_{}_transition", mode_name(from), mode_name(to));
            if let Some(last) = self.transition.replace(name.clone()) {
                post(out, format!("{last}_e"), Emitter::Ship);
            }
            post(out, format!("{name}_b"), Emitter::Ship);
        }
        out.push(Call::State("FrameShiftState", state_of_mode(to)));
        orbital_cruise(to, out);
        post(out, format!("{}_enter", mode_name(to)), Emitter::Ship);
        post(out, format!("{}_enter_from_{}", mode_name(to), mode_name(from)), Emitter::Ship);
        self.mode = to;
    }

    /// engines_on_off.
    fn engines(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        if v.engines_on != self.engines_on {
            self.engines_on = v.engines_on;
            out.push(Call::State("Drives", if v.engines_on { "On" } else { "Off" }));
            self.off_timer = ENGINES_OFF_S;
            if v.engines_on && self.mode == 1 {
                self.drive_loops(true, out);
            }
        }
        if !self.engines_on && self.drive_loops {
            self.off_timer -= v.dt;
            if self.off_timer < 0.0 {
                self.drive_loops(false, out);
            }
        }
    }

    /// motion_rtpcs, power_heat_contacts_rtpcs, player_inside_switch, supercruise_rtpcs' heat, the charge's game parameters.
    fn motion(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        let m = v.motion;
        let dt = v.dt;
        let s = &mut self.smooth;
        let rel_in = if v.flight_assist { m.speed } else { m.speed_fa_off };
        s.relative = if (rel_in - s.relative).abs() > SLEW_RELATIVE { slew(s.relative, rel_in, SLEW_RELATIVE, dt) } else { rel_in };
        s.accel = slew(s.accel, m.accel_relative, SLEW, dt);
        s.directional = slew(s.directional, m.accel_directional, SLEW, dt);
        s.target = slew(s.target, m.target_speed, SLEW, dt);
        s.rot = slew(s.rot, m.speed_rot, SLEW_ROT, dt);
        let g = |n: &'static str, x: f64| Call::Gp(n, x as f32);
        out.extend([
            g("spaceship_speed_self", m.speed),
            g("spaceship_speed_physical_self", m.speed_physical),
            g("spaceship_speed_relative_self", s.relative.clamp(-RELATIVE_MOST, RELATIVE_MOST)),
            g("spaceship_accel_self", m.accel),
            g("spaceship_accel_normalised_self", m.accel_normalised),
            g("spaceship_accel_directional_normalised_self", m.accel_dir_normalised),
            g("spaceship_accel_relative_smooth_self", s.accel),
            g("spaceship_accel_relative_slow_self", s.accel),
            g("spaceship_accel_directional_self", s.directional),
            g("spaceship_target_speed_relative_self", s.target),
            g("spaceship_pitch_self", m.pitch),
            g("spaceship_yaw_self", m.yaw),
            g("spaceship_roll_self", m.roll),
            g("spaceship_speed_rot_self", m.speed_rot),
            g("spaceship_speed_rot_self_smooth", s.rot),
            g("spaceship_outOfControlYaw_self", m.out_of_control_yaw),
            g("power_systems_self", v.pips[0]),
            g("power_engines_self", v.pips[1]),
            g("power_weapons_self", v.pips[2]),
            g("num_contacts", 0.0),
            g("num_hostile_ships", 0.0),
            Call::Gp("PlayerInsideAndEmitterAttached", INSIDE),
        ]);
        // Heat in percent (power_heat_contacts_rtpcs; supercruise_rtpcs sends the same ratio x 100 and its change a second).
        let pct = v.heat * 100.0;
        out.push(g("spaceship_heat_self", pct));
        if self.mode == 2 && dt > 0.0 {
            out.push(g("spaceship_heat_change_ratio_self", self.heat_pct.map_or(0.0, |p| (pct - p) / dt)));
        }
        self.heat_pct = Some(pct);
        // fsd_charge: the charge and the countdown's seconds by kind.
        match v.drive {
            Drive::Charging(_) => out.push(g("supercruise_charge_self", v.drive.charge_share().unwrap_or(0.0))),
            Drive::JumpCharging(_) | Drive::Departing(_) => {
                out.push(g("hyperdrive_charge_self", v.drive.charge_share().unwrap_or(0.0)));
                if let Some(left) = v.drive.countdown() {
                    out.push(g("hyperdrive_countdown_self", left));
                }
            }
            _ => {}
        }
    }

    /// boost.
    fn boost(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        const NAMES: [&str; 4] = ["idle", "warmup", "firing", "cooldown"];
        if v.boost != self.boost {
            self.boost = v.boost;
            for (i, n) in NAMES.iter().enumerate() {
                if i as u8 == v.boost {
                    post(out, format!("boost_{n}_b_sidewinder_internal"), Emitter::Ship);
                }
            }
            for (i, n) in NAMES.iter().enumerate() {
                if i as u8 != v.boost {
                    post(out, format!("boost_{n}_e_sidewinder_internal"), Emitter::Ship);
                }
            }
        }
        // The refused flag (+0x86) set posts boost_failed_b; cleared, boost_failed_e (the frame after: the flag is a
        // press's). The banks' failed_e is empty.
        if v.boost_refused {
            post(out, "boost_failed_b_sidewinder_internal", Emitter::Ship);
            self.refused_shown = true;
        } else if self.refused_shown {
            post(out, "boost_failed_e_sidewinder_internal", Emitter::Ship);
            self.refused_shown = false;
        }
    }

    /// carried_00, carried_01, landing_gear, cockpit_lights_fuelscoop_reboot (lights), cargo_scoop, silent_running,
    /// canopy_crack, dock_glide_stress (the glide's state).
    fn cockpit(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        if v.flight_assist != self.flight_assist {
            self.flight_assist = v.flight_assist;
            // The ship's pair: _b when assist goes OFF (state On), _e when it comes back ON (state Off). The cockpit's beep
            // the other way round (measured: carried_01).
            if v.flight_assist {
                post(out, "FlightAssist_e", Emitter::Ship);
                out.push(Call::State("FlightAssist", "Off"));
                post(out, "Cockpit_FlightAssist_b", Emitter::CockpitOrigin);
            } else {
                post(out, "FlightAssist_b", Emitter::Ship);
                out.push(Call::State("FlightAssist", "On"));
                post(out, "Cockpit_FlightAssist_e", Emitter::CockpitOrigin);
            }
        }
        if v.gear_down != self.gear_down {
            self.gear_down = v.gear_down;
            out.push(Call::State("LandingGear", if v.gear_down { "Deployed" } else { "Retracted" }));
            post(out, if v.gear_down { "LandingGear_Deployed" } else { "LandingGear_Retracted" }, Emitter::CockpitOrigin);
        }
        if v.lights != self.lights {
            self.lights = v.lights;
            post(out, if v.lights { "Cockpit_LightsOn" } else { "Cockpit_LightsOff" }, Emitter::CockpitOrigin);
        }
        if v.scoop != self.scoop {
            self.scoop = v.scoop;
            post(out, if v.scoop { "CargoScoop_Deployed" } else { "CargoScoop_Retracted" }, Emitter::CockpitOrigin);
            if v.scoop {
                post(out, "CargoScoop_b", Emitter::CockpitOrigin);
            } else {
                // (Closed ends the loop and both warnings.)
                for n in ["CargoScoop_e", "CargoScoopWarning_e", "CargoScoopRefineryWarning_e"] {
                    post(out, n, Emitter::CockpitOrigin);
                }
            }
        }
        if v.silent != self.silent {
            self.silent = v.silent;
            post(out, if v.silent { "SilentRunning_b" } else { "SilentRunning_e" }, Emitter::CockpitOrigin);
        }
        // The cockpit machine: Breaching sets CockpitCrack_Blowout, Canopy Breached posts CockpitBreach_None (both at the
        // breach here: the flight has one canopy flag, not the machine's stages).
        if v.canopy_breached && !self.canopy_breached {
            out.push(Call::State("CockpitCrack", "CockpitCrack_Blowout"));
            post(out, "CockpitBreach_None", Emitter::CockpitOrigin);
        }
        self.canopy_breached = v.canopy_breached;
        if v.glide != self.glide {
            self.glide = v.glide;
            out.push(Call::State("GlideState", if v.glide { "Glide" } else { "NotGlide" }));
        }
    }

    /// cockpit_atmosphere: the atmosphere audio component's rule (`FUN_144295130`, set-up `FUN_144294a50`, the setter
    /// `FUN_144295590`): pressurised while the oxygen mask is off and (the cockpit is whole, or one of three flags whose
    /// meaning is not found, taken as off); else depressurised. Set at the start and whenever it changes. (The setter also
    /// sets a game parameter `cockpit_atmosphere`, 1 or 0, that no shipped object reads.)
    fn cockpit_atmosphere(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        let pressurised = !v.oxygen_mask && !v.canopy_breached;
        if self.pressurised != Some(pressurised) {
            self.pressurised = Some(pressurised);
            out.push(Call::State("cockpit_atmosphere", if pressurised { "pressurised" } else { "depressurised" }));
        }
    }

    /// fsd_charge, fsd_countdown, fsd_cancel_and_point_of_no_return, voice_fsd_lines, fsd_cooldown_masslock_cues.
    fn fsd(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        let before = self.drive.unwrap_or(v.drive);
        let (was, now) = (charge_kind(before), charge_kind(v.drive));
        let charge = |k: u8| if k == 2 { "hyperdrive" } else { "supercruise" };
        if now != 0 && now != was {
            if was != 0 {
                post(out, format!("{}_charge_internal_e", charge(was)), Emitter::Ship);
            }
            post(out, format!("{}_charge_internal_b", charge(now)), Emitter::Ship);
            post(out, "VoxV01_FrameshiftDriveCharging", Emitter::Voice);
            post(out, "Cockpit_FrameShiftInitiated", Emitter::CockpitOrigin);
            self.held = false;
        }
        if now == 0 && was != 0 {
            post(out, "hyperdrive_charge_internal_e", Emitter::Ship);
            post(out, "supercruise_charge_internal_e", Emitter::Ship);
            // Past the point a press can no longer cancel (the flight's: supercruise entered, the tunnel begun) it is the
            // point of no return; otherwise the charge was cancelled (by the key) or refused.
            let engaged = match was {
                1 => v.drive == Drive::Supercruise,
                _ => v.drive == Drive::Tunnel || v.mode == 3,
            };
            if engaged {
                post(out, format!("{}_PointOfNoReturn_internal_b", charge(was)), Emitter::Ship);
            } else {
                // (supercruise_cancel_internal is no event of the banks: the supercruise cancel posts nothing.)
                if was == 2 {
                    post(out, "hyperdrive_cancel_internal", Emitter::Ship);
                }
                post(out, if v.jump_pressed { "Cockpit_FrameShiftCancel" } else { "Cockpit_FrameShiftUnsuccessful" }, Emitter::CockpitOrigin);
            }
        }
        // A press the drive refused (it stayed as it was, not charging).
        if v.jump_pressed && was == 0 && now == 0 && before == v.drive && matches!(v.drive, Drive::Ready | Drive::Arriving(_) | Drive::Cooldown(_) | Drive::Arrived(_)) {
            post(out, "Cockpit_FrameShiftUnsuccessful", Emitter::CockpitOrigin);
        }
        // The charge complete and waiting (its condition at the charge's end): "ready to engage".
        let held = v.drive.held().is_some();
        if held && !self.held {
            post(out, "VoxV01_FrameshiftReadyToEngage", Emitter::Voice);
        }
        self.held = held;
        let (tick, voice) = self.countdown.step(v.drive.countdown());
        if tick {
            post(out, if charge_kind(v.drive) == 1 { "supercruise_count_internal" } else { "hyperdrive_count_internal" }, Emitter::Ship);
        }
        for n in voice {
            post(out, *n, Emitter::Voice);
        }
        match cooldown_progress(v.drive) {
            Some(p) => {
                if !self.cooldown {
                    post(out, "Cockpit_FsdCooldown_b", Emitter::CockpitOrigin);
                }
                self.cooldown = true;
                out.push(Call::Gp("GUI_FSDCooldownProgess", p as f32));
            }
            None if self.cooldown => {
                self.cooldown = false;
                post(out, "Cockpit_FsdCooldown_e", Emitter::CockpitOrigin);
                out.push(Call::Gp("GUI_FSDCooldownProgess", 0.0));
            }
            None => {}
        }
        if v.safe_drop != self.safe_drop {
            match v.safe_drop {
                Some(true) => post(out, "Message_b_SafetoDisengage", Emitter::CockpitOrigin),
                Some(false) => post(out, "Message_b_SlowDown", Emitter::CockpitOrigin),
                None => {}
            }
            self.safe_drop = v.safe_drop;
        }
        self.drive = Some(v.drive);
    }

    /// carried_02 (heat), carried_05 (hull), carried_06 (shields).
    fn damage(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        if let Some(line) = self.heat.step(v.heat, v.dt) {
            post(out, line, Emitter::Voice);
        }
        let mut lines = Vec::new();
        self.hull.step(v.hull, &mut lines);
        for l in lines {
            post(out, l, Emitter::Voice);
        }
        if v.shields_up != self.shields_up {
            self.shields_up = v.shields_up;
            if v.shields_up {
                post(out, "VoxV01_ShieldsOnline", Emitter::Voice);
                post(out, "Reform_ShieldGenerator", Emitter::Ship);
            } else {
                post(out, "VoxV01_ShieldsOffline", Emitter::Voice);
                // Down with its power on: broken; with it off (silent running, the switch, no fuel): turned off.
                post(out, if v.shields_powered { "Break_ShieldGenerator" } else { "TurnOff_ShieldGenerator" }, Emitter::Ship);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ShipView {
        ShipView::at_rest()
    }

    fn posts(calls: &[Call]) -> Vec<String> {
        calls.iter().filter_map(|c| if let Call::Post(n, _) = c { Some(n.clone()) } else { None }).collect()
    }

    fn run(t: &mut Triggers, v: &ShipView) -> Vec<String> {
        let mut out = Vec::new();
        t.step(v, &mut out);
        posts(&out)
    }

    #[test]
    fn heat_lines_by_threshold_cooldown_and_alternation() {
        let mut h = HeatVoice::default();
        assert_eq!(h.step(1.1, 0.1), None);
        assert_eq!(h.step(1.25, 0.1), Some("VoxV01_WarningTemperatureCritical"));
        // The 30 s cooldown.
        assert_eq!(h.step(1.3, 10.0), None);
        // Past 1.5: at once "taking heat damage", whatever the cooldown.
        assert_eq!(h.step(1.6, 1.0), Some("VoxV01_WarningTakingHeatDamage"));
        assert_eq!(h.step(1.6, 29.0), None);
        // Then every 30 s the other line, alternating.
        assert_eq!(h.step(1.6, 1.0), Some("VoxV01_WarningTemperatureCritical"));
        assert_eq!(h.step(1.6, 30.0), Some("VoxV01_WarningTakingHeatDamage"));
        assert_eq!(h.step(1.6, 30.0), Some("VoxV01_WarningTemperatureCritical"));
        // Under 1.5 the alternation clears: back over it, "taking heat damage" first again.
        assert_eq!(h.step(1.3, 0.1), None);
        assert_eq!(h.step(1.6, 0.1), Some("VoxV01_WarningTakingHeatDamage"));
        // At 1.2 exactly nothing (the rule is 1.2 < h).
        let mut h = HeatVoice::default();
        assert_eq!(h.step(1.2, 100.0), None);
    }

    #[test]
    fn hull_lines_are_once_per_ship() {
        let mut t = Triggers::default();
        let mut v = view();
        run(&mut t, &v);
        v.hull = 0.9;
        assert!(run(&mut t, &v).contains(&"VoxV01_HullIntegrityCompromised".to_string()));
        v.hull = 0.8;
        assert!(run(&mut t, &v).contains(&"VoxV01_TakingDamage".to_string()));
        v.hull = 0.7;
        assert!(!run(&mut t, &v).iter().any(|p| p.contains("Hull") || p.contains("TakingDamage")));
        v.hull = 0.2;
        assert_eq!(run(&mut t, &v).iter().filter(|p| p.contains("Critical")).count(), 1);
        v.hull = 0.1;
        assert!(!run(&mut t, &v).iter().any(|p| p.contains("Hull")));
        // Getting out and back in keeps the flags (the same ship); a new ship clears them.
        let mut out = Vec::new();
        t.teardown(&mut out);
        v.hull = 0.05;
        run(&mut t, &v);
        v.hull = 0.04;
        assert!(!run(&mut t, &v).iter().any(|p| p.contains("Hull")));
        t.new_ship();
        run(&mut t, &v);
        v.hull = 0.03;
        let p = run(&mut t, &v);
        assert!(p.contains(&"VoxV01_HullIntegrityCompromised".to_string()) && p.contains(&"VoxV01_HullIntegrityCritical".to_string()));
    }

    #[test]
    fn countdown_ticks_and_numbers() {
        let mut c = Countdown::default();
        let mut ticks = 0;
        let mut said = Vec::new();
        let mut left = 8.0;
        while left > 0.0 {
            let (t, v) = c.step(Some(left));
            ticks += t as i32;
            said.extend(v.iter().copied());
            left -= 1.0 / 60.0;
        }
        // From 8 s: a tick at each whole second's drop (7..0), the voice at 6, 4, 3, 2, 1 (5 says nothing).
        assert_eq!(ticks, 8);
        assert_eq!(said, ["VoxV01_Number5", "VoxV01_Number4", "VoxV01_Number3", "VoxV01_Number2", "VoxV01_Number1", "VoxV01_Engage"]);
        // None running resets it; the flight's 5 s countdown never says "five".
        assert_eq!(c.step(None), (false, &[][..]));
        let (t, v) = c.step(Some(5.0));
        assert!(!t && v.is_empty());
    }

    #[test]
    fn flight_assist_pairs_have_opposite_senses() {
        let mut t = Triggers::default();
        let mut v = view();
        run(&mut t, &v);
        v.flight_assist = false;
        let mut out = Vec::new();
        t.step(&v, &mut out);
        let p = posts(&out);
        assert!(p.contains(&"FlightAssist_b".to_string()) && p.contains(&"Cockpit_FlightAssist_e".to_string()));
        assert!(out.contains(&Call::State("FlightAssist", "On")));
        v.flight_assist = true;
        let mut out = Vec::new();
        t.step(&v, &mut out);
        let p = posts(&out);
        assert!(p.contains(&"FlightAssist_e".to_string()) && p.contains(&"Cockpit_FlightAssist_b".to_string()));
        assert!(out.contains(&Call::State("FlightAssist", "Off")));
    }

    fn states(calls: &[Call], group: &str) -> Vec<&'static str> {
        calls.iter().filter_map(|c| match c { Call::State(g, s) if *g == group => Some(*s), _ => None }).collect()
    }

    #[test]
    fn cockpit_atmosphere_follows_the_mask_and_the_canopy() {
        let mut t = Triggers::default();
        let mut v = view();
        let mut out = Vec::new();
        t.step(&v, &mut out);
        assert_eq!(states(&out, "cockpit_atmosphere"), ["pressurised"], "set at the start");
        out.clear();
        t.step(&v, &mut out);
        assert!(states(&out, "cockpit_atmosphere").is_empty(), "only on a change");
        v.canopy_breached = true;
        out.clear();
        t.step(&v, &mut out);
        assert_eq!(states(&out, "cockpit_atmosphere"), ["depressurised"]);
        v.canopy_breached = false;
        v.oxygen_mask = true;
        out.clear();
        t.step(&v, &mut out);
        assert!(states(&out, "cockpit_atmosphere").is_empty(), "still depressurised: the mask is on");
        v.oxygen_mask = false;
        out.clear();
        t.step(&v, &mut out);
        assert_eq!(states(&out, "cockpit_atmosphere"), ["pressurised"]);
    }

    #[test]
    fn modes_post_their_loops_and_transitions() {
        let mut t = Triggers::default();
        let mut v = view();
        let start = run(&mut t, &v);
        assert_eq!(start.iter().filter(|p| p.ends_with("_b_sidewinder_internal")).count(), 8 + 9);
        assert_eq!(start.iter().filter(|p| p.ends_with("_SharedCockpit_b")).count(), 9);
        v.mode = 2;
        let p = run(&mut t, &v);
        assert!(p.contains(&"normalflight_supercruise_transition_b".to_string()));
        assert!(p.contains(&"supercruise_enter_from_normalflight".to_string()));
        let mut calls = Vec::new();
        v.mode = 1;
        t.step(&v, &mut calls);
        v.mode = 2;
        calls.clear();
        t.step(&v, &mut calls);
        assert_eq!(states(&calls, "OrbitalCruise"), ["Off"], "supercruise's audio starts with orbital cruise off");
        assert_eq!(p.iter().filter(|p| p.ends_with("_e_sidewinder_internal")).count(), 8 + 9);
        assert_eq!(p.iter().filter(|p| p.ends_with("_Supercruise_Cockpit_b")).count(), 9);
        assert!(!p.iter().any(|p| p.ends_with("_transition_e")), "the first transition ends nothing");
        v.mode = 3;
        let p = run(&mut t, &v);
        let (e, b) = (p.iter().position(|p| p == "normalflight_supercruise_transition_e"), p.iter().position(|p| p == "supercruise_hyperspace_transition_b"));
        assert!(e.is_some() && b.is_some() && e < b, "the last transition's end, then the new one's begin: {p:?}");
        v.mode = 2;
        let p = run(&mut t, &v);
        assert!(p.contains(&"supercruise_hyperspace_transition_e".to_string()) && p.contains(&"hyperspace_supercruise_transition_b".to_string()));
        // A change with no transition event (hyperspace to normal space) leaves the handle as it is.
        v.mode = 3;
        run(&mut t, &v);
        v.mode = 1;
        assert!(!run(&mut t, &v).iter().any(|p| p.contains("_transition_")));
    }

    #[test]
    fn engines_off_stops_the_loops_three_seconds_later() {
        let mut t = Triggers::default();
        let mut v = view();
        run(&mut t, &v);
        v.engines_on = false;
        v.dt = 1.0;
        let mut out = Vec::new();
        t.step(&v, &mut out);
        assert!(out.contains(&Call::State("Drives", "Off")));
        assert!(!posts(&out).iter().any(|p| p.ends_with("_e_sidewinder_internal")));
        run(&mut t, &v);
        run(&mut t, &v);
        assert_eq!(run(&mut t, &v).iter().filter(|p| p.ends_with("_e_sidewinder_internal")).count(), 8);
    }

    #[test]
    fn boost_states_in_order() {
        let mut t = Triggers::default();
        let mut v = view();
        run(&mut t, &v);
        v.boost = 1;
        let p = run(&mut t, &v);
        assert_eq!(p[0], "boost_warmup_b_sidewinder_internal");
        assert_eq!(p.len(), 4);
        v.boost = 2;
        assert_eq!(run(&mut t, &v)[0], "boost_firing_b_sidewinder_internal");
        v.boost = 3;
        assert_eq!(run(&mut t, &v)[0], "boost_cooldown_b_sidewinder_internal");
        v.boost = 0;
        assert_eq!(run(&mut t, &v)[0], "boost_idle_b_sidewinder_internal");
    }

    #[test]
    fn the_charge_its_voice_and_the_point_of_no_return() {
        let mut t = Triggers::default();
        let mut v = view();
        v.gear_down = false;
        run(&mut t, &v);
        v.drive = Drive::Charging(0.0);
        let p = run(&mut t, &v);
        assert!(p.contains(&"supercruise_charge_internal_b".to_string()) && p.contains(&"VoxV01_FrameshiftDriveCharging".to_string()));
        v.drive = Drive::Supercruise;
        v.mode = 2;
        let p = run(&mut t, &v);
        assert!(p.contains(&"supercruise_PointOfNoReturn_internal_b".to_string()) && p.contains(&"supercruise_charge_internal_e".to_string()));
        // A jump's charge cancelled by the key.
        v.drive = Drive::JumpCharging(0.0);
        run(&mut t, &v);
        v.drive = Drive::Supercruise;
        v.jump_pressed = true;
        let p = run(&mut t, &v);
        assert!(p.contains(&"hyperdrive_cancel_internal".to_string()) && p.contains(&"Cockpit_FrameShiftCancel".to_string()));
    }

    #[test]
    fn motion_mapping_from_a_ship_state() {
        let f = Flight { vel: [0.0, 0.0, -110.0], acc: [0.0, 0.0, -20.0], omega: [0.5, 0.2, -1.0], top_speed: 220.0, accel_fwd: 40.0, back_share: 0.5, throttle: -0.5 };
        let m = motion_in(&f);
        assert!((m.speed - 50.0).abs() < 1e-9);
        assert!((m.speed_physical - 110.0).abs() < 1e-9);
        assert!((m.accel - 20.0).abs() < 1e-9);
        assert!((m.accel_relative - 50.0).abs() < 1e-9);
        assert!((m.target_speed + 25.0).abs() < 1e-9);
        assert!((m.yaw - 0.2f64.to_degrees()).abs() < 1e-9);
        assert_eq!((m.pitch, m.roll), (0.5, -1.0));
        // The slew: min(|d|, r sqrt(|d|) dt).
        assert!((slew(0.0, 100.0, 3.0, 0.1) - 3.0).abs() < 1e-9);
        assert_eq!(slew(0.0, 0.01, 3.0, 1.0), 0.01);
        assert_eq!(slew(5.0, 7.0, 0.0, 0.1), 7.0);
        // speed_relative snaps under a 5-point gap and is clamped to 199.9.
        let mut t = Triggers::default();
        let mut v = view();
        run(&mut t, &v);
        v.motion.speed = 300.0;
        let mut out = Vec::new();
        for _ in 0..600 {
            out.clear();
            t.step(&v, &mut out);
        }
        assert!(out.contains(&Call::Gp("spaceship_speed_relative_self", 199.9)));
    }
}
