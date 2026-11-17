//! When Elite posts each of the ship's sound events and how it sets each game parameter, state and switch: the rules of
//! `re/elite/frea_export/sidewinder_audio/triggers.json` (its entry ids name each rule below), as pure code. The ship's
//! state goes in as a [`ShipView`] once a frame; what Elite's audio objects would call comes out as [`Call`]s, which
//! `mod.rs` hands to the mixer. Nothing here reads Bevy or the mixer, so the rules are tested alone.
//!
//! Only entries whose certainty is `proven`, `proven, one link inferred` or `measured` are here. What the ship's state
//! cannot drive (no such state in the flight) is listed in `mod.rs`.
//!
//! **The engine's inputs (T5d-6)** are Elite's audio data producer's rules (`Producer`: 0x14415ebc0, 0x14415deb0; the
//! notes' "T5e" and its table "for the sound's writer"), in Elite's units: percents of the full-power and the current top
//! speed, their changes over the physics time simulated, the spin's shares of the speed-limited turn rates (high-passed
//! with assist off), "stationary", `engine_mass_modifier`; then the audio object's slews on the physics time
//! (0x14415c5c0, 0x14415c3d0). The flight's numbers come in as `facts::SoundMotion`: nothing of the flight is worked out
//! here. Not built, for want of an identified source: the producer's state flag 0x37 (the "stopped" test, and the slewed
//! physics time it selects); the predicted speed (0x14415ebc0 line 126: Elite takes it whenever input +0x95 is 1, assist
//! on, or flag 0x48 is clear: last update's world velocity plus the physics time x the controller's ship-axis acceleration
//! +0x710, less a term under flag 0x37; that acceleration's source, body slot 0x170, is not identified, so the flight's
//! own speed stands in, the same number in steady flight); producer+0xbc8 (which zeroes the turn lanes); the object at
//! producer+0x688 (which snaps "stationary" to 1); the component's byte D+0x30 (which snaps speed_relative); the gear
//! component's answer that keeps the full-power top speed slewing (`Producer::slewed_full_top`); and the Euler
//! decomposition of the spin (0x1405e0db0: taken as the spin's own components).

use elite_sim::drive::Drive;

use super::super::facts::SoundMotion;

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
    /// A state machine's cue emitter (`mod.rs` `ShipSound::cue_emitter`: one per machine and bone set, placed at its bones).
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

/// `audio_engine_class`: the state AudioEngineClass of the Sidewinder's stock thrusters. Proven (triggers.json
/// `audio_engine_class`, T5e): the audio update's engine block (0x14415a9a0) hands the setter (0x14415ab60) the string at
/// +0xe8 of the thruster module's data (0x142ec6b60), and the record of `Int_Engine_Size2_Class1`, the ship's stock
/// thrusters, holds "Class1" there (grade C, the read; B, the record). Measured too (T5d-4): the full-throttle drive layer
/// 02ad84c3 sits exactly at Class1's pitch in the user's recordings (+200 cents on its chain; a spectral score of 0.82
/// against 0.30 for the next state), and the layer 2e468dbb agrees weakly at Class1's +300.
pub const ENGINE_CLASS: &str = "Class1";

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

/// `motion_rtpcs`: the producer's motion block (Elite's audio data D+0x00..+0x44, T5e items 4, 5), in Elite's units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionIn {
    /// D+0x00 `spaceship_accel_self`: the change of `speed` over the physics time (percent a second).
    pub accel: f64,
    /// D+0x04 `spaceship_speed_self`: the speed in percent of the full-power top speed (speed_relative's with assist off).
    pub speed: f64,
    /// D+0x08: the speed in percent of the current top speed, negative backwards (speed_relative's with assist on).
    pub relative: f64,
    /// D+0x0c `spaceship_speed_physical_self`: the body's speed in percent of the full-power top speed.
    pub speed_physical: f64,
    /// D+0x10, D+0x14: `accel` over the current block's forward acceleration, its size and signed.
    pub accel_normalised: f64,
    pub accel_dir_normalised: f64,
    /// D+0x18: the change of `relative`'s size over the physics time (smoothed into `_relative_smooth` and `_relative_slow`).
    pub accel_relative: f64,
    /// D+0x1c: the change of `relative` over the physics time (smoothed into `spaceship_accel_directional_self`).
    pub accel_directional: f64,
    /// D+0x20: the forward demand in percent (smoothed into `spaceship_target_speed_relative_self`).
    pub target_speed: f64,
    /// D+0x2c `spaceship_speed_stationary_self` (`speed_stationary`).
    pub stationary: f64,
    /// D+0x34, +0x38, +0x3c, +0x40, +0x44: the spin's shares of the speed-limited turn rates, and the largest.
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
    pub speed_rot: f64,
    pub out_of_control_yaw: f64,
}

/// D+0x08 turns negative while the ship moves backwards faster than this (m/s; 0x14415ebc0 line 224).
pub const BACKWARDS_MPS: f64 = 1.0;
/// `speed_stationary`: with assist off the target is 1 while `speed` moves by more than this in an update; the slew's rates
/// rising and falling (0x14415ebc0 lines 308-345).
pub const STATIONARY_MOVE: f64 = 0.1;
pub const STATIONARY_UP: f64 = 3.0;
pub const STATIONARY_DOWN: f64 = 0.5;
/// Assist off, each turn lane is high-passed: `clamp((raw - slew(raw, 1.0)) x 2.5, +-2)` (0x14415deb0 lines 33-105).
pub const HIGH_PASS_SLEW: f64 = 1.0;
pub const HIGH_PASS_GAIN: f64 = 2.5;
pub const HIGH_PASS_MOST: f64 = 2.0;
/// The pitch and roll lanes' clamp (0x14415deb0 lines 159-180).
pub const LANE_MOST: f64 = 1.99;
/// The full-power top speed's slew (0x14415ebc0 lines 153-203, its producer+0xaf0): it slews at this rate (square-root law,
/// frame time) while `1 - kept / new` is at most `FULL_TOP_DROP` or at least `FULL_TOP_RISE` (the constants at
/// 0x14556e468, 0x144e29758, 0x144deaa5c, read from the exe), and snaps to the new value otherwise.
pub const FULL_TOP_DROP: f64 = -0.1;
pub const FULL_TOP_RISE: f64 = 0.1;
pub const FULL_TOP_SLEW: f64 = 3.0;

/// The ship audio data producer's motion rules (`motion_rtpcs`, `speed_stationary`: 0x14415ebc0 speeds, accelerations,
/// target and stationary; 0x14415deb0 rotation), with what they keep from one update to the next. Made afresh whenever the
/// audio starts (a boarding), so its first update has nothing to differ from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Producer {
    /// `speed` and `relative` at the last update (None before the first: its derivatives are 0).
    last: Option<(f64, f64)>,
    /// D+0x00 and D+0x1c as last worked out: held through an update with no physics time.
    accel: f64,
    directional: f64,
    /// D+0x2c as slewed (None before the first update: it starts at its target).
    stationary: Option<f64>,
    /// The high-pass's slewed lanes (pitch, yaw, roll), moved only while assist is off, set to the last outputs whenever
    /// assist changes.
    high_pass: [f64; 3],
    /// Flight assist as the turn lanes read it (producer+0xbd5: taken from the input at the first update, then at the end of
    /// every update in which it changed; None before the first update).
    assist: Option<bool>,
    /// The full-power top speed as slewed (producer+0xaf0; None before the first update: it starts at the flight's).
    full_top: Option<f64>,
}

impl Producer {
    /// One update: the flight's numbers this frame, the forward demand (-1..1), flight assist and the frame's real seconds.
    pub fn update(&mut self, f: &SoundMotion, throttle: f64, flight_assist: bool, dt: f64) -> MotionIn {
        let dtp = f.physics_dt;
        let [_, _, vz] = f.vel;
        let s = f.vel.iter().map(|v| v * v).sum::<f64>().sqrt();
        let percent_of = |top: f64| if top > 0.0 { 100.0 * s / top } else { 0.0 };
        let speed = percent_of(self.slewed_full_top(f.full_top, dt));
        // (Elite's nose is +z, ours -z.)
        let relative = if -vz < -BACKWARDS_MPS { -percent_of(f.top) } else { percent_of(f.top) };
        let (accel, accel_relative, directional) = match self.last {
            Some((s0, r0)) if dtp > 0.0 => ((speed - s0) / dtp, (relative.abs() - r0.abs()) / dtp, (relative - r0) / dtp),
            Some(_) => (self.accel, 0.0, self.directional),
            None => (0.0, 0.0, 0.0),
        };
        let normalised = if f.accel_fwd != 0.0 { accel / f.accel_fwd } else { 0.0 };
        let stationary = self.speed_stationary(speed, flight_assist, dt);
        self.last = Some((speed, relative));
        self.accel = accel;
        self.directional = directional;
        // The spin's share of each speed-limited turn rate; assist off, only its changes (the high-pass). The lanes read
        // assist as the producer last stored it (0x14415deb0 tests +0xbd5, which the update changes only after it).
        let mut lane = [0, 1, 2].map(|k| if f.share_limits[k] > 0.0 { f.omega[k] / f.share_limits[k] } else { 0.0 });
        let out_of_control_yaw = lane[1];
        let assist = *self.assist.get_or_insert(flight_assist);
        if !assist {
            for (l, hp) in lane.iter_mut().zip(self.high_pass.iter_mut()) {
                *hp = slew(*hp, *l, HIGH_PASS_SLEW, dt);
                *l = ((*l - *hp) * HIGH_PASS_GAIN).clamp(-HIGH_PASS_MOST, HIGH_PASS_MOST);
            }
        }
        let (pitch, yaw, roll) = (lane[0].clamp(-LANE_MOST, LANE_MOST), lane[1], lane[2].clamp(-LANE_MOST, LANE_MOST));
        // A change of assist (0x144145200 lines 289-298, after the lanes): stored, and the high-pass set to this update's
        // outputs D+0x34, +0x38, +0x3c, so a change adds nothing.
        if assist != flight_assist {
            self.assist = Some(flight_assist);
            self.high_pass = [pitch, yaw, roll];
        }
        MotionIn {
            accel,
            speed,
            relative,
            // (The mod's flight has one velocity: the controller's and the body's speeds are the same.)
            speed_physical: speed,
            accel_normalised: normalised.abs(),
            accel_dir_normalised: normalised,
            accel_relative,
            accel_directional: directional,
            target_speed: 100.0 * throttle,
            stationary,
            pitch,
            yaw,
            roll,
            speed_rot: lane.iter().map(|l| l.clamp(-1.0, 1.0).abs()).fold(0.0, f64::max),
            out_of_control_yaw,
        }
    }

    /// The full-power top speed the percents are taken of: the flight's `now`, reached by `FULL_TOP_SLEW` after a change of
    /// 10 % or more (the gear's or the scoop's halving), snapped after a smaller one. The exe also slews while the gear's
    /// component answers its slot 0x30 (0x14307e6e0: some leg answers its own slot 0x38, not identified): taken as never.
    fn slewed_full_top(&mut self, now: f64, dt: f64) -> f64 {
        let v = match self.full_top {
            Some(kept) => {
                let change = if now != 0.0 { 1.0 - kept / now } else { 0.0 };
                if change <= FULL_TOP_DROP || change >= FULL_TOP_RISE { slew(kept, now, FULL_TOP_SLEW, dt) } else { now }
            }
            None => now,
        };
        self.full_top = Some(v);
        v
    }

    /// `speed_stationary`: the target 1 with assist on; off, 1 while `speed` moved by more than `STATIONARY_MOVE` since the
    /// last update, else 0 (the "stopped" flag, state flag 0x37, not identified: never set here); slewed at frame time.
    fn speed_stationary(&mut self, speed: f64, flight_assist: bool, dt: f64) -> f64 {
        let moving = self.last.is_some_and(|(s0, _)| (speed - s0).abs() > STATIONARY_MOVE);
        let target = if flight_assist || moving { 1.0 } else { 0.0 };
        let now = match self.stationary {
            Some(v) => slew(v, target, if v < target { STATIONARY_UP } else { STATIONARY_DOWN }, dt),
            None => target,
        };
        self.stationary = Some(now);
        now
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
    /// The flight's numbers the motion rules read (`facts::SoundMotion`) and the forward demand (-1..1: Elite's input +0x10).
    pub motion: SoundMotion,
    pub throttle: f64,
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
            motion: SoundMotion { physics_dt: 1.0 / 60.0, ..stock(2.0, [0.0; 3]) },
            throttle: 0.0,
            pips: [2.0; 3],
        }
    }
}

/// The old feeder's input, kept only so `voices.rs`'s tests (another task's file) still build: `motion_in` makes the flight's
/// numbers of a ship flying at `vel` with `top_speed` as both top speeds and no physics time (the throttle is `ShipView`'s
/// now). TO GO when `voices.rs`'s tests use `stock` (its writer's change no longer calls it: unused until then).
#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flight {
    pub vel: [f64; 3],
    pub acc: [f64; 3],
    pub omega: [f64; 3],
    pub top_speed: f64,
    pub accel_fwd: f64,
    pub back_share: f64,
    pub throttle: f64,
}

#[cfg(test)]
#[allow(dead_code)]
pub fn motion_in(f: &Flight) -> SoundMotion {
    SoundMotion { full_top: f.top_speed, top: f.top_speed, accel_fwd: f.accel_fwd, vel: f.vel, omega: f.omega, mass_k: 1.0, ..SoundMotion::default() }
}

/// The stock ship's flight numbers (its mass with a full tank, the gear up) at `eng` ENG pips, flying at `vel` (ship axes,
/// m/s) with no spin and no physics time, for the sound's tests.
#[cfg(test)]
pub fn stock(eng: f64, vel: [f64; 3]) -> SoundMotion {
    use elite_sim::data::{sidewinder_block, FUEL_TANK_T, JOURNAL_UNLADEN_MASS, SIDEWINDER};
    let mass = JOURNAL_UNLADEN_MASS + FUEL_TANK_T + SIDEWINDER.reserve_fuel;
    let (block, full_top) = sidewinder_block(eng, mass);
    SoundMotion::new(&block, full_top, vel, [0.0; 3], 0, mass)
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
    /// The producer's motion rules and the audio object's smoothed values (None before the first update: they start where
    /// their inputs are).
    producer: Producer,
    smooth: Option<Smooth>,
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

/// player_location: the player's audio context sets state group player_location whenever its value changes
/// (`FUN_1441e45d0`, the ids made by `FUN_1441dc7d0`): unknown to start with; near a station's docking component the station's
/// states (inside_station, inside_docking_bay, the outpost's and planet port's bays, nearby_outpost by flag 0x53); with no
/// station's component near it falls to outside_station (0x1441e4a3a..0x1441e4a76: flag 0x53 clear -> outside_station).
/// The mod has no stations, so outside_station from the start, in normal space, supercruise and on a planet alike (the
/// function tells them apart only by the context's mode 0xb, which keeps the last value). Its only effect on the shipped
/// objects: bus b2e99de6's EQ 36b637f5 (a 400 Hz low-pass and -18 dB) is bypassed (BypassFX0).
fn player_location(out: &mut Vec<Call>) {
    out.push(Call::State("player_location", "outside_station"));
}

/// on_foot_location: the on-foot audio context sets OnFootLocation (`FUN_1441e5190`): NotOnFoot whenever the player is not
/// on foot (its last branch: the static "NotOnFoot", set when it differs from the last one set), the on-foot places otherwise. A
/// seated pilot is never on foot. (The same function sets the game parameter inside_vacuum, which no shipped object reads.)
fn on_foot_location(out: &mut Vec<Call>) {
    out.push(Call::State("OnFootLocation", "NotOnFoot"));
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
        // audio_engine_class: set when the audio object starts (Elite sets it whenever the thrusters' class differs from
        // the one last set; the mod's thrusters never change).
        out.push(Call::State("AudioEngineClass", ENGINE_CLASS));
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
        // FlightAssist On while assist is on (the set-up 0x14414d320 reads the input's byte +0x95, non-zero with assist on).
        out.push(Call::State("LandingGear", if v.gear_down { "Deployed" } else { "Retracted" }));
        out.push(Call::State("FlightAssist", if v.flight_assist { "On" } else { "Off" }));
        out.push(Call::State("GlideState", if v.glide { "Glide" } else { "NotGlide" }));
        self.cockpit_atmosphere(v, out);
        player_location(out);
        on_foot_location(out);
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

    /// motion_rtpcs, speed_stationary, engine_mass_modifier, power_heat_contacts_rtpcs, player_inside_switch,
    /// supercruise_rtpcs' heat, the charge's game parameters.
    fn motion(&mut self, v: &ShipView, out: &mut Vec<Call>) {
        let m = self.producer.update(&v.motion, v.throttle, v.flight_assist, v.dt);
        let dt = v.dt;
        // The audio object's slews run on the physics time (0x144143830 hands 0x14415c5c0 and 0x14415c3d0 D+0x1ac): none
        // moves in a frame with no physics step. speed_relative reads D+0x08 with assist on, D+0x04 with it off.
        let dtp = v.motion.physics_dt;
        let rel_in = if v.flight_assist { m.relative } else { m.speed };
        let s = match self.smooth {
            None => Smooth { relative: rel_in, accel: m.accel_relative, directional: m.accel_directional, target: m.target_speed, rot: m.speed_rot },
            Some(s) => Smooth {
                relative: if (rel_in - s.relative).abs() > SLEW_RELATIVE { slew(s.relative, rel_in, SLEW_RELATIVE, dtp) } else { rel_in },
                accel: slew(s.accel, m.accel_relative, SLEW, dtp),
                directional: slew(s.directional, m.accel_directional, SLEW, dtp),
                target: slew(s.target, m.target_speed, SLEW, dtp),
                rot: slew(s.rot, m.speed_rot, SLEW_ROT, dtp),
            },
        };
        self.smooth = Some(s);
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
            g("spaceship_speed_stationary_self", m.stationary),
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
            // engine_mass_modifier: the thrusters' top-speed mass multiplier, every frame (0x14415ab60).
            g("engine_mass_modifier", v.motion.mass_k),
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
            // carried_00 (T5e): assist ON posts the ship's _b and state On, OFF its _e and state Off (0x144143830 on D+0x49,
            // the input's +0x95, non-zero with assist on; the recording's beeps agree). carried_01: the cockpit's pair the same
            // way round (measured).
            if v.flight_assist {
                post(out, "FlightAssist_b", Emitter::Ship);
                out.push(Call::State("FlightAssist", "On"));
                post(out, "Cockpit_FlightAssist_b", Emitter::CockpitOrigin);
            } else {
                post(out, "FlightAssist_e", Emitter::Ship);
                out.push(Call::State("FlightAssist", "Off"));
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

    /// carried_00 as T5e corrected it: assist ON is state On and FlightAssist_b, OFF state Off and FlightAssist_e (the
    /// start too), each once per change; the cockpit's pair the same way round (carried_01).
    #[test]
    fn flight_assist_follows_elite_both_ways() {
        let mut t = Triggers::default();
        let mut v = view();
        let mut out = Vec::new();
        t.step(&v, &mut out);
        assert_eq!(states(&out, "FlightAssist"), ["On"], "the start with assist on");
        assert!(!posts(&out).iter().any(|p| p.contains("FlightAssist")));
        v.flight_assist = false;
        out.clear();
        t.step(&v, &mut out);
        let p = posts(&out);
        assert!(p.contains(&"FlightAssist_e".to_string()) && p.contains(&"Cockpit_FlightAssist_e".to_string()), "{p:?}");
        assert!(!p.iter().any(|p| p.ends_with("FlightAssist_b")));
        assert_eq!(states(&out, "FlightAssist"), ["Off"]);
        out.clear();
        t.step(&v, &mut out);
        assert!(!posts(&out).iter().any(|p| p.contains("FlightAssist")) && states(&out, "FlightAssist").is_empty(), "once per change");
        v.flight_assist = true;
        out.clear();
        t.step(&v, &mut out);
        let p = posts(&out);
        assert!(p.contains(&"FlightAssist_b".to_string()) && p.contains(&"Cockpit_FlightAssist_b".to_string()), "{p:?}");
        assert!(!p.iter().any(|p| p.ends_with("FlightAssist_e")));
        assert_eq!(states(&out, "FlightAssist"), ["On"]);
        let mut t = Triggers::default();
        out.clear();
        t.step(&ShipView { flight_assist: false, ..view() }, &mut out);
        assert_eq!(states(&out, "FlightAssist"), ["Off"], "the start with assist off");
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

    const TICK: f64 = 1.0 / 60.0;

    /// A game parameter's last value in the calls.
    fn gp(calls: &[Call], name: &str) -> f64 {
        let x = calls.iter().rev().find_map(|c| match c {
            Call::Gp(n, x) if *n == name => Some(*x),
            _ => None,
        });
        f64::from(x.unwrap_or_else(|| panic!("{name} is not set")))
    }

    /// `frames` frames of the same view; the last frame's calls.
    fn settle(t: &mut Triggers, v: &ShipView, frames: usize) -> Vec<Call> {
        let mut out = Vec::new();
        for _ in 0..frames {
            out.clear();
            t.step(v, &mut out);
        }
        out
    }

    fn near(got: f64, want: f64, what: &str) {
        assert!((got - want).abs() < 1e-3 * want.abs().max(1.0), "{what}: {got} against {want}");
    }

    /// A ship flying steadily at `vel` (ship axes) at `eng` pips, a tick every frame at 60 frames a second.
    fn flying(eng: f64, vel: [f64; 3], fa: bool, throttle: f64) -> ShipView {
        ShipView { flight_assist: fa, throttle, motion: SoundMotion { physics_dt: TICK, ..stock(eng, vel) }, ..view() }
    }

    /// The slew: min(|d|, r sqrt(|d|) dt); r <= 0 snaps.
    #[test]
    fn the_slew() {
        assert!((slew(0.0, 100.0, 3.0, 0.1) - 3.0).abs() < 1e-9);
        assert_eq!(slew(0.0, 0.01, 3.0, 1.0), 0.01);
        assert_eq!(slew(5.0, 7.0, 0.0, 0.1), 7.0);
        assert_eq!(slew(5.0, 7.0, 3.0, 0.0), 5.0);
    }

    /// Full throttle on each pip setting (T5e's table): speed_self and speed_physical are percents of the full-power top
    /// speed (45.5, 72.7 and 100 at 0, 2, 4 ENG pips), speed_relative 100 with assist on (the current top speed's percent)
    /// and speed_self with it off; the target 100; nothing changes, so the accelerations are 0.
    #[test]
    fn full_throttle_on_each_pip_setting() {
        for (eng, share) in [(0.0, 100.0 / 220.0), (2.0, 160.0 / 220.0), (4.0, 1.0)] {
            let top = stock(eng, [0.0; 3]).top;
            for fa in [true, false] {
                let out = settle(&mut Triggers::default(), &flying(eng, [0.0, 0.0, -top], fa, 1.0), 30);
                let what = format!("ENG {eng}, assist {fa}");
                near(gp(&out, "spaceship_speed_self"), 100.0 * share, &what);
                near(gp(&out, "spaceship_speed_physical_self"), 100.0 * share, &what);
                near(gp(&out, "spaceship_speed_relative_self"), if fa { 100.0 } else { 100.0 * share }, &what);
                near(gp(&out, "spaceship_target_speed_relative_self"), 100.0, &what);
                for n in ["spaceship_accel_self", "spaceship_accel_relative_smooth_self", "spaceship_accel_directional_self", "spaceship_accel_normalised_self"] {
                    assert_eq!(gp(&out, n), 0.0, "{what}: {n}");
                }
            }
        }
    }

    /// Reverse: speed_relative turns negative only past 1 m/s backwards; speed_self stays a size; the target follows the
    /// demand's sign (100 x the forward demand, no back share).
    #[test]
    fn reverse_and_the_backwards_threshold() {
        let top = stock(2.0, [0.0; 3]).top;
        let out = settle(&mut Triggers::default(), &flying(2.0, [0.0, 0.0, 0.6 * top], true, -0.6), 30);
        near(gp(&out, "spaceship_speed_relative_self"), -60.0, "backwards");
        near(gp(&out, "spaceship_speed_self"), 60.0 * 160.0 / 220.0, "backwards");
        near(gp(&out, "spaceship_target_speed_relative_self"), -60.0, "the demand");
        let out = settle(&mut Triggers::default(), &flying(2.0, [0.0, 0.0, 0.5], true, 0.0), 3);
        assert!(gp(&out, "spaceship_speed_relative_self") > 0.0, "0.5 m/s backwards is not past the threshold");
    }

    /// A boost: past the top speed the percents pass 100 (320 m/s is 145 % of the full-power top speed); speed_relative's
    /// clamp holds it at 199.9 (ENG 2: 200 % of 160 m/s), reached at the slew's rate 5 over a gap of more than 5.
    #[test]
    fn a_boost_passes_one_hundred_percent() {
        let boost = 320.0 * stock(4.0, [0.0; 3]).full_top / 220.0;
        let out = settle(&mut Triggers::default(), &flying(4.0, [0.0, 0.0, -boost], true, 1.0), 30);
        near(gp(&out, "spaceship_speed_self"), 100.0 * 320.0 / 220.0, "boost");
        near(gp(&out, "spaceship_speed_relative_self"), 100.0 * 320.0 / 220.0, "boost");
        let mut t = Triggers::default();
        settle(&mut t, &flying(2.0, [0.0, 0.0, -100.0], true, 1.0), 3);
        let out = settle(&mut t, &flying(2.0, [0.0, 0.0, -boost], true, 1.0), 1);
        let first = gp(&out, "spaceship_speed_relative_self");
        let from = 100.0 * 100.0 / stock(2.0, [0.0; 3]).top;
        let gap: f64 = 200.0 - from;
        near(first, from + SLEW_RELATIVE * gap.sqrt() * TICK, "the slew's first step");
        let out = settle(&mut t, &flying(2.0, [0.0, 0.0, -boost], true, 1.0), 600);
        assert_eq!(gp(&out, "spaceship_speed_relative_self"), RELATIVE_MOST as f32 as f64);
    }

    /// Turning: each lane is the spin's share of the speed-limited turn rate (pitch's, yaw's, roll's rate x the turn curve's
    /// blue zone at s / Vcur, roll's too), 1 at the full rate and 0.5 at half; pitch and roll clamped to 1.99, speed_rot the
    /// largest share clamped to 1, outOfControlYaw the yaw share.
    #[test]
    fn a_turn_at_the_full_rate_and_at_half() {
        let top = stock(2.0, [0.0; 3]).top;
        let base = flying(2.0, [0.0, 0.0, -top], true, 1.0);
        let w = base.motion.share_limits;
        // At the top speed the share's divisors are the raw rates x the turn curve's b (0.35 at ENG 2), roll's too.
        let k = elite_sim::data::thrusters_mass_k(elite_sim::data::JOURNAL_UNLADEN_MASS + 2.3);
        for (got, rate) in w.iter().zip([39.0f64, 17.0, 110.0]) {
            near(*got, (rate * k).to_radians() * 0.35, "a divisor");
        }
        for (axis, name) in [(0, "spaceship_pitch_self"), (1, "spaceship_yaw_self"), (2, "spaceship_roll_self")] {
            for share in [1.0, 0.5, 2.5] {
                let mut omega = [0.0; 3];
                omega[axis] = share * w[axis];
                let v = ShipView { motion: SoundMotion { omega, ..base.motion }, ..base };
                let out = settle(&mut Triggers::default(), &v, 30);
                let want = if axis == 1 { share } else { share.min(LANE_MOST) };
                near(gp(&out, name), want, name);
                near(gp(&out, "spaceship_speed_rot_self"), share.min(1.0), name);
                near(gp(&out, "spaceship_speed_rot_self_smooth"), share.min(1.0), name);
                near(gp(&out, "spaceship_outOfControlYaw_self"), if axis == 1 { share } else { 0.0 }, name);
            }
        }
    }

    /// Assist off, only the spin's changes sound: a turn begun at the full rate is (1 - slew(0, 1, 1.0, dt)) x 2.5 clamped
    /// to 2 at once (pitch 1.99), and fades to 0 as the high-pass catches up (2 s at the slew's rate); outOfControlYaw is the
    /// raw share. With assist on the lanes are the shares again. A new start forgets the high-pass.
    #[test]
    fn assist_off_high_passes_the_turn() {
        let top = stock(2.0, [0.0; 3]).top;
        let mut v = flying(2.0, [0.0, 0.0, -top], false, 1.0);
        let mut t = Triggers::default();
        settle(&mut t, &v, 5);
        let w = v.motion.share_limits;
        v.motion.omega = [w[0], 0.5 * w[1], 0.0];
        let out = settle(&mut t, &v, 1);
        near(gp(&out, "spaceship_pitch_self"), LANE_MOST, "the step");
        near(gp(&out, "spaceship_yaw_self"), ((0.5 - slew(0.0, 0.5, HIGH_PASS_SLEW, TICK)) * HIGH_PASS_GAIN).min(HIGH_PASS_MOST), "the step");
        near(gp(&out, "spaceship_outOfControlYaw_self"), 0.5, "raw");
        let out = settle(&mut t, &v, 180);
        assert!(gp(&out, "spaceship_pitch_self").abs() < 1e-6 && gp(&out, "spaceship_speed_rot_self").abs() < 1e-6, "settled to 0");
        near(gp(&out, "spaceship_outOfControlYaw_self"), 0.5, "raw");
        v.flight_assist = true;
        let out = settle(&mut t, &v, 2);
        near(gp(&out, "spaceship_pitch_self"), 1.0, "assist on");
        let mut out = Vec::new();
        t.teardown(&mut out);
        v.flight_assist = false;
        let out = settle(&mut t, &v, 1);
        near(gp(&out, "spaceship_pitch_self"), LANE_MOST, "a new start: the high-pass from 0");
    }

    /// The lanes' pitch, yaw and roll this frame.
    fn lanes(out: &[Call]) -> [f64; 3] {
        ["spaceship_pitch_self", "spaceship_yaw_self", "spaceship_roll_self"].map(|n| gp(out, n))
    }

    /// A change of assist reseeds the high-pass with the update's own outputs (0x144145200 lines 289-298, after the lanes,
    /// which read assist as stored): switched off in a full-rate turn, the switch's frame still gives the share (1) and
    /// every frame after it 0; switched on in a settled assist-off turn, the switch's frame gives 0 and the next the share;
    /// switched off at rest after an earlier assist-off turn, nothing at all (the old memory is gone).
    #[test]
    fn a_change_of_assist_reseeds_the_high_pass() {
        let top = stock(2.0, [0.0; 3]).top;
        let mut v = flying(2.0, [0.0, 0.0, -top], true, 1.0);
        let w = v.motion.share_limits;
        let turning = [w[0], 0.5 * w[1], 0.0];
        // Off in a full-rate turn.
        let mut t = Triggers::default();
        v.motion.omega = turning;
        settle(&mut t, &v, 30);
        v.flight_assist = false;
        let first = lanes(&settle(&mut t, &v, 1));
        near(first[0], 1.0, "the switch's frame: the outputs as assist was");
        near(first[1], 0.5, "the switch's frame: the outputs as assist was");
        for frame in 0..120 {
            let out = settle(&mut t, &v, 1);
            assert!(lanes(&out).iter().all(|l| l.abs() < 1e-9) && gp(&out, "spaceship_speed_rot_self").abs() < 1e-9, "frame {frame}: {:?}", lanes(&out));
        }
        // On again in the (settled) assist-off turn: 0 in the switch's frame, the shares after it.
        v.flight_assist = true;
        let first = lanes(&settle(&mut t, &v, 1));
        assert!(first.iter().all(|l| l.abs() < 1e-9), "{first:?}");
        let next = lanes(&settle(&mut t, &v, 1));
        near(next[0], 1.0, "assist on");
        near(next[1], 0.5, "assist on");
        // Off at rest after an earlier assist-off turn (begun, then switched on mid-transient, then stopped).
        let mut t = Triggers::default();
        v.flight_assist = false;
        v.motion.omega = [0.0; 3];
        settle(&mut t, &v, 5);
        v.motion.omega = turning;
        settle(&mut t, &v, 30);
        v.flight_assist = true;
        settle(&mut t, &v, 2);
        v.motion.omega = [0.0; 3];
        settle(&mut t, &v, 60);
        v.flight_assist = false;
        for frame in 0..120 {
            let out = settle(&mut t, &v, 1);
            assert!(lanes(&out).iter().all(|l| l.abs() < 1e-9), "frame {frame}: {:?} with no spin", lanes(&out));
        }
    }

    /// The full-power top speed slews after the gear's or the scoop's halving (sqrt law, rate 3, frame time) until within
    /// 10 %, then snaps: at 100 m/s speed_self follows that slew (its change a tick at most what the slew gives, never the
    /// whole halving's 45 points at once), and the last 10 % snap is the exe's own. At rest nothing moves.
    #[test]
    fn the_full_power_top_speed_slews_after_the_gear_or_the_scoop() {
        let speed_of = |full: f64| 100.0 * 100.0 / full;
        for (gear, scoop) in [(true, false), (false, true), (true, true)] {
            let k = elite_sim::deployables::speed_factor(gear, scoop);
            let up = stock(2.0, [0.0, 0.0, -100.0]);
            let down = SoundMotion { full_top: up.full_top * k, top: up.top * k, ..up };
            for (from, to) in [(up, down), (down, up)] {
                let mut t = Triggers::default();
                let mut v = ShipView { motion: SoundMotion { physics_dt: TICK, ..from }, ..view() };
                settle(&mut t, &v, 10);
                v.motion = SoundMotion { physics_dt: TICK, ..to };
                // The exe's rule worked alongside: kept, its speed_self, the largest change a tick while slewing.
                let mut kept = from.full_top;
                let mut snapped = false;
                for frame in 0..600 {
                    let was = speed_of(kept);
                    let change = 1.0 - kept / to.full_top;
                    let slewing = change <= FULL_TOP_DROP || change >= FULL_TOP_RISE;
                    kept = if slewing { slew(kept, to.full_top, FULL_TOP_SLEW, TICK) } else { to.full_top };
                    let out = settle(&mut t, &v, 1);
                    let what = format!("gear {gear}, scoop {scoop}, {} to {}, frame {frame}", from.full_top, to.full_top);
                    near(gp(&out, "spaceship_speed_self"), speed_of(kept), &what);
                    near(gp(&out, "spaceship_accel_self"), (speed_of(kept) - was) / TICK, &what);
                    if slewing {
                        // sqrt law: at most 3 sqrt(110 m/s) / 60 = 0.53 m/s of full-power speed a tick (26 %/s at most, against the snap's 2,700).
                        assert!(gp(&out, "spaceship_accel_self").abs() < 50.0, "{what}: {}", gp(&out, "spaceship_accel_self"));
                    } else if !snapped && kept == to.full_top {
                        snapped = true;
                        // (The snap: under 10 % of speed_self.)
                        assert!((speed_of(kept) - was).abs() <= 0.11 * speed_of(kept), "{what}");
                    }
                }
                assert!(snapped && kept == to.full_top, "reached");
            }
            // At rest the halving moves nothing.
            let mut t = Triggers::default();
            let rest = stock(2.0, [0.0; 3]);
            let mut v = ShipView { motion: SoundMotion { physics_dt: TICK, ..rest }, ..view() };
            settle(&mut t, &v, 5);
            v.motion = SoundMotion { full_top: rest.full_top * k, top: rest.top * k, ..v.motion };
            for _ in 0..300 {
                let out = settle(&mut t, &v, 1);
                assert_eq!((gp(&out, "spaceship_speed_self"), gp(&out, "spaceship_accel_self")), (0.0, 0.0));
            }
        }
    }

    /// At rest and landed (ticks running, no motion): every motion input 0, stationary 1 with assist on and 0 with it off
    /// (not moving), the mass modifier the thrusters' multiplier.
    #[test]
    fn at_rest_and_landed() {
        for fa in [true, false] {
            let v = ShipView { flight_assist: fa, ..view() };
            let out = settle(&mut Triggers::default(), &v, 30);
            for n in ["spaceship_speed_self", "spaceship_speed_relative_self", "spaceship_accel_self", "spaceship_pitch_self", "spaceship_speed_rot_self", "spaceship_target_speed_relative_self"] {
                assert_eq!(gp(&out, n), 0.0, "{n}");
            }
            assert_eq!(gp(&out, "spaceship_speed_stationary_self"), if fa { 1.0 } else { 0.0 });
            near(gp(&out, "engine_mass_modifier"), v.motion.mass_k, "mass");
            assert!((1.004..1.006).contains(&gp(&out, "engine_mass_modifier")));
        }
    }

    /// "Stationary" with assist off: 1 while the speed moves more than 0.1 % of the full-power top speed an update, rising
    /// at 3.0 and falling at 0.5 (frame time); with assist on always 1.
    #[test]
    fn stationary_follows_the_speeds_changes_with_assist_off() {
        let mut v = view();
        v.flight_assist = false;
        let mut t = Triggers::default();
        settle(&mut t, &v, 2);
        // 45 m/s^2: 0.75 m/s a tick, 0.34 % of 221 m/s.
        let full = v.motion.full_top;
        let mut speed = 0.0;
        let mut out = Vec::new();
        for _ in 0..3 {
            speed += 45.0 * TICK;
            v.motion.vel = [0.0, 0.0, -speed];
            out = settle(&mut t, &v, 1);
        }
        assert!(100.0 * 45.0 * TICK / full > STATIONARY_MOVE);
        let mut want = 0.0;
        for _ in 0..3 {
            want = slew(want, 1.0, STATIONARY_UP, TICK);
        }
        near(gp(&out, "spaceship_speed_stationary_self"), want, "rising");
        let out = settle(&mut t, &v, 1);
        near(gp(&out, "spaceship_speed_stationary_self"), slew(want, 0.0, STATIONARY_DOWN, TICK), "falling: the speed held");
    }

    /// The accelerations are the percents' changes over the physics time: the same motion (45 m/s^2 forward from rest, 60
    /// ticks a second) seen at 20, 60 and 144 frames a second gives the same inputs in every frame with a tick, holds
    /// accel_self and gives accel_relative 0 in a frame without one, and its smoothed values (slewed on the physics time)
    /// meet; the first update after a start gives 0, never a spike.
    #[test]
    fn derivatives_are_the_same_at_any_frame_rate() {
        let a = 45.0;
        let mut ends = Vec::new();
        for fps in [20u32, 60, 144] {
            let mut t = Triggers::default();
            let mut v = view();
            v.throttle = 1.0;
            let (mut ticks, mut vel) = (0u32, 0.0);
            let mut out = Vec::new();
            // accel_self as last worked out (0 from the first update until a frame with a tick).
            let mut held = None;
            for frame in 1..=(5 * fps) {
                let now = frame * 60 / fps;
                let n = now - ticks;
                for _ in 0..n {
                    vel += a * TICK;
                }
                ticks = now;
                v.dt = 1.0 / f64::from(fps);
                v.motion = SoundMotion { physics_dt: f64::from(n) * TICK, ..stock(2.0, [0.0, 0.0, -vel]) };
                out.clear();
                t.step(&v, &mut out);
                let (accel, what) = (gp(&out, "spaceship_accel_self"), format!("{fps} fps, frame {frame}"));
                let rate = 100.0 * a / v.motion.full_top;
                match held {
                    None => {
                        assert_eq!(accel, 0.0, "{what}: the first update");
                        held = Some(0.0);
                    }
                    Some(_) if n > 0 => {
                        near(accel, rate, &what);
                        near(gp(&out, "spaceship_accel_directional_normalised_self"), rate / v.motion.accel_fwd, &what);
                        held = Some(rate);
                    }
                    Some(h) => near(accel, h, &format!("{what}: held")),
                }
            }
            ends.push((gp(&out, "spaceship_accel_relative_smooth_self"), gp(&out, "spaceship_accel_directional_self"), gp(&out, "spaceship_speed_relative_self")));
        }
        let want = 100.0 * a / stock(2.0, [0.0; 3]).top;
        for (fps, (smooth, dir, rel)) in [20, 60, 144].iter().zip(&ends) {
            // (The sqrt-law slew reaches its input in 2 sqrt(x) / 3 s, 3.5 s for these 28 %/s: 5 s is past it.)
            near(*smooth, want, &format!("{fps} fps: accel_relative_smooth"));
            near(*dir, want, &format!("{fps} fps: accel_directional"));
            near(*rel, ends[1].2, &format!("{fps} fps: speed_relative"));
        }
    }

    /// Nothing non-finite from a zero top speed, a zero full-power top speed, no physics time, no turn rate or no forward
    /// acceleration, moving and spinning.
    #[test]
    fn nothing_non_finite_from_zeros() {
        let zero = SoundMotion { full_top: 0.0, top: 0.0, accel_fwd: 0.0, share_limits: [0.0; 3], physics_dt: 0.0, vel: [3.0, -2.0, -50.0], omega: [0.4, -0.2, 1.0], mass_k: 1.0 };
        for fa in [true, false] {
            let mut t = Triggers::default();
            for m in [zero, SoundMotion { physics_dt: TICK, ..zero }, SoundMotion { vel: [0.0; 3], ..zero }] {
                let out = settle(&mut t, &ShipView { flight_assist: fa, motion: m, ..view() }, 3);
                for c in &out {
                    if let Call::Gp(n, x) = c {
                        assert!(x.is_finite(), "{n} = {x}");
                    }
                }
            }
        }
    }

    /// Supercruise (the drive loops are off there): the motion inputs go on from the flight's numbers as in normal space
    /// (T5e is silent on what Elite feeds there: the mod's behaviour kept).
    #[test]
    fn supercruise_keeps_the_motion_inputs() {
        let top = stock(2.0, [0.0; 3]).top;
        let v = ShipView { mode: 2, drive: Drive::Supercruise, gear_down: false, ..flying(2.0, [0.0, 0.0, -top], true, 1.0) };
        let out = settle(&mut Triggers::default(), &v, 30);
        near(gp(&out, "spaceship_speed_relative_self"), 100.0, "supercruise");
        assert!(out.iter().all(|c| !matches!(c, Call::Gp(_, x) if !x.is_finite())));
    }
}
