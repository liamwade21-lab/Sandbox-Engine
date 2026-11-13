//! What the skater does and which of Skate 3's clips shows it: a small state machine over the board's physics (`physics.rs`)
//! in place of Skate 3's own state graphs, and a player of the clips (`clips.rs`) with crossfades.
//!
//! - **Riding**: standing still, rolling, tucked at speed; leaning into a turn blends towards Skate 3's carve poses
//!   (`MED_CRV_POSE_1` leaning left, `_6` right) by the deck's tilt.
//! - **Pushing**: into a push, a stroke while push is held, back on the board; each stroke's speed is given when the foot
//!   comes down (`PUSH_AT` of the clip).
//! - **Braking**, **powersliding**: into, held, out.
//! - **A jump**: the crouch while the stick is held down, then the pop (`OLLIE_*_G`, the physics' launch on its first tick),
//!   then either the ollie's air or a flip trick: its turning (`T_*_CYC`, played once at the trick's own pace) and its catch
//!   (`T_*_OUT`). A trick counts as caught a little into its catch; landing before that is a bail.
//! - **Landing**: clean or heavy by how hard. **A bail**: flailing until he's down and slowed, then getting up.

use std::sync::Arc;

use bevy::math::{Quat, Vec3};
use se_core::skate3::{Clips, Sample};

use super::input::Trick;
use super::physics::Phase;

/// The clips by name, once.
pub struct ClipIndex {
    pub clips: Clips,
    names: std::collections::HashMap<String, usize>,
}

impl ClipIndex {
    pub fn new(clips: Clips) -> Self {
        let names = clips.clips.iter().enumerate().map(|(i, c)| (c.name.to_ascii_uppercase(), i)).collect();
        ClipIndex { clips, names }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.names.get(&name.to_ascii_uppercase()).copied()
    }

    pub fn bones(&self) -> usize {
        self.clips.bones as usize
    }

    /// A clip's pose at a time (frames interpolated; a looping clip wraps).
    pub fn sample(&self, clip: usize, t: f32, out: &mut [Sample]) {
        let c = &self.clips.clips[clip];
        let n = self.bones();
        let frames = c.frames.max(1) as usize;
        let len = c.seconds();
        let t = if c.looping && len > 0.0 { t.rem_euclid(len) } else { t.clamp(0.0, len) };
        let f = t * c.fps;
        let i = (f.floor() as usize).min(frames - 1);
        let j = if i + 1 < frames { i + 1 } else if c.looping { 0 } else { i };
        let k = f - i as f32;
        let (a, b) = (c.frame(i, n), c.frame(j, n));
        for (o, (a, b)) in out.iter_mut().zip(a.iter().zip(b)) {
            *o = mix(*a, *b, k);
        }
    }
}

/// Two samples mixed: scale and translation straight, the rotation along the shorter way.
pub fn mix(a: Sample, b: Sample, k: f32) -> Sample {
    let qa = Quat::from_xyzw(a[3], a[4], a[5], a[6]);
    let mut qb = Quat::from_xyzw(b[3], b[4], b[5], b[6]);
    if qa.dot(qb) < 0.0 {
        qb = -qb;
    }
    let q = qa.lerp(qb, k).normalize();
    let l = |i: usize| a[i] + (b[i] - a[i]) * k;
    [l(0), l(1), l(2), q.x, q.y, q.z, q.w, l(7), l(8), l(9)]
}

/// A clip playing.
#[derive(Clone, Copy, Debug)]
struct Play {
    clip: usize,
    t: f32,
    rate: f32,
}

/// The clip player: what plays, and the pose it is fading from.
pub struct Animator {
    index: Arc<ClipIndex>,
    now: Play,
    from: Vec<Sample>,
    fade: f32,
    fade_total: f32,
    pose: Vec<Sample>,
    scratch: Vec<Sample>,
}

impl Animator {
    pub fn new(index: Arc<ClipIndex>, first: &str) -> Self {
        let n = index.bones();
        let clip = index.find(first).unwrap_or(0);
        let identity = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let mut a = Animator { index, now: Play { clip, t: 0.0, rate: 1.0 }, from: vec![identity; n], fade: 0.0, fade_total: 0.0, pose: vec![identity; n], scratch: vec![identity; n] };
        a.refresh();
        a
    }

    pub fn index(&self) -> &ClipIndex {
        &self.index
    }

    /// Play a clip from its start, fading from the pose as it is over `fade` seconds, at `rate` times its own speed. A clip
    /// that is missing leaves things as they are.
    pub fn play(&mut self, name: &str, fade: f32, rate: f32) {
        let Some(clip) = self.index.find(name) else { return };
        self.from.copy_from_slice(&self.pose);
        self.fade = fade;
        self.fade_total = fade;
        self.now = Play { clip, t: 0.0, rate };
    }

    /// Play it unless it is already the one playing.
    pub fn keep(&mut self, name: &str, fade: f32) {
        if self.playing() != name {
            self.play(name, fade, 1.0);
        }
    }

    pub fn playing(&self) -> &str {
        &self.index.clips.clips[self.now.clip].name
    }

    /// How far through it is, 0..1 (a looping clip's count of times round goes on past 1).
    pub fn progress(&self) -> f32 {
        let len = self.index.clips.clips[self.now.clip].seconds();
        if len <= 0.0 { 1.0 } else { self.now.t / len }
    }

    /// A clip that doesn't loop has reached its end.
    pub fn done(&self) -> bool {
        !self.index.clips.clips[self.now.clip].looping && self.progress() >= 1.0
    }

    pub fn advance(&mut self, dt: f32) {
        self.now.t += dt * self.now.rate;
        self.fade = (self.fade - dt).max(0.0);
    }

    /// This frame's pose, with a lean towards the carve poses (-1 left .. 1 right, by the share given).
    pub fn pose(&mut self, carve: Option<(f32, f32)>) -> &[Sample] {
        self.refresh();
        if let Some((lean, share)) = carve {
            let name = if lean < 0.0 { "MED_CRV_POSE_1" } else { "MED_CRV_POSE_6" };
            let k = (lean.abs() * share).clamp(0.0, 1.0);
            if let (Some(c), true) = (self.index.find(name), k > 0.001) {
                let mut lean_pose = std::mem::take(&mut self.scratch);
                self.index.sample(c, 0.0, &mut lean_pose);
                // The trajectory and the body; the board's own bones keep the physics' tilt.
                for (b, (p, l)) in self.pose.iter_mut().zip(&lean_pose).enumerate() {
                    if (1..25).contains(&b) {
                        *p = mix(*p, *l, k);
                    }
                }
                self.scratch = lean_pose;
            }
        }
        &self.pose
    }

    fn refresh(&mut self) {
        let mut pose = std::mem::take(&mut self.pose);
        self.index.sample(self.now.clip, self.now.t, &mut pose);
        if self.fade > 0.0 && self.fade_total > 0.0 {
            let k = self.fade / self.fade_total;
            let k = k * k * (3.0 - 2.0 * k);
            for (p, f) in pose.iter_mut().zip(&self.from) {
                *p = mix(*p, *f, k);
            }
        }
        self.pose = pose;
    }
}

/// A trick's clips: the pop, the board turning, the catch, and how long the turning takes.
pub struct TrickClips {
    pub cyc: &'static str,
    pub out: &'static str,
    pub cyc_seconds: f32,
}

pub fn trick_clips(t: Trick) -> Option<TrickClips> {
    let c = |cyc, out, cyc_seconds| Some(TrickClips { cyc, out, cyc_seconds });
    match t {
        Trick::Ollie => None,
        Trick::Kickflip => c("T_LOW_KICK_CYC1", "T_KICKFLIP_LOW_DCATCH_0_OUT1", 0.22),
        Trick::Heelflip => c("T_LOW_HEEL_CYC1", "T_HEELFLIP_LOW_DCATCH_OUT1", 0.22),
        Trick::Shuvit => c("T_POPSHUVIT_L_CYC", "T_POPSHUVIT_LOW_UFLIP_OUT1", 0.3),
        Trick::FsShuvit => c("T_FSPOPSHUVIT_L_CYC", "T_FSPOPSHUVIT_LOW_UFLIP_OUT1", 0.3),
        Trick::Flip360 => c("T_360FLIP_L_CYC", "T_360FLIP_LOW_DCATCH_OUT1", 0.36),
        Trick::Laser => c("T_LASERFLIP_L_CYC", "T_LASERFLIP_LOW_DCATCH_OUT1", 0.36),
    }
}

/// Where in a push's clip the foot comes down and gives the board its speed.
const PUSH_AT: f32 = 0.55;
/// A trick counts as caught this far into its catch.
const CAUGHT_AT: f32 = 0.45;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    Ride,
    PushInto,
    PushStroke,
    PushOut,
    BrakeInto,
    Brake,
    BrakeOut,
    SlideInto,
    Slide,
    SlideOut,
    Crouch,
    /// The pop's clip; the trick (if any) follows.
    Pop(Trick),
    TrickTurn(Trick),
    TrickCatch(Trick),
    Air,
    Land,
    Bail,
    GetUp,
}

/// The controller: from the controls and the board's state to the clips, the board's drive, and what was done.
pub struct Controller {
    pub state: State,
    pub anim: Animator,
    /// The push stroke's speed has been given in this clip.
    pushed: bool,
    /// The trick in the air has been caught (or was an ollie).
    pub caught: bool,
    /// The trick last landed, for the log and the HUD.
    pub last_trick: Option<(Trick, bool)>,
}

/// What the controller saw this tick.
pub struct Seen {
    pub phase: Phase,
    pub speed: f32,
    pub fakie: bool,
    pub push: bool,
    pub brake: bool,
    pub slide_side: f32,
    pub crouch: bool,
    pub pop: Option<(Trick, f32)>,
}

/// What it asks of the board this tick.
#[derive(Default)]
pub struct Asked {
    pub push: bool,
    pub pushing: bool,
    pub brake: bool,
    pub slide: f32,
    pub pop: Option<f32>,
    pub popped_trick: Option<Trick>,
}

impl Controller {
    pub fn new(index: Arc<ClipIndex>) -> Self {
        Controller { state: State::Ride, anim: Animator::new(index, "R_STAND_IDLE2_N_0_CYC"), pushed: false, caught: true, last_trick: None }
    }

    fn ride_clip(speed: f32) -> &'static str {
        if speed < 0.3 {
            "R_STAND_IDLE2_N_0_CYC"
        } else if speed > 7.5 {
            "R_IDLE_SPEEDTUCK_N_0_CYC"
        } else {
            "R_IDLE_RIDE_N_0_CYC"
        }
    }

    fn go(&mut self, s: State, clip: &str, fade: f32, rate: f32) {
        self.state = s;
        self.anim.play(clip, fade, rate);
        self.pushed = false;
    }

    /// One tick before the board's physics: what to ask of it.
    pub fn before(&mut self, s: &Seen, dt: f32) -> Asked {
        let mut a = Asked::default();
        let on_ground = s.phase == Phase::Ground;
        // A pop is taken on the ground from riding, a crouch, a push (Skate 3 ollies out of a push), braking or a landing;
        // not mid-powerslide or in a bail.
        if on_ground && matches!(self.state, State::Ride | State::Crouch | State::PushInto | State::PushStroke | State::PushOut | State::BrakeInto | State::Brake | State::BrakeOut | State::Land | State::SlideOut) {
            if let Some((trick, crouched)) = s.pop {
                let strength = (crouched / 0.45).clamp(0.0, 1.0);
                let clip = if strength > 0.5 { "OLLIE_HIGH_G" } else { "OLLIE_LOW_G" };
                self.go(State::Pop(trick), clip, 0.06, 1.0);
                self.caught = trick == Trick::Ollie;
                a.pop = Some(strength);
                a.popped_trick = Some(trick);
                self.anim.advance(dt);
                return a;
            }
        }
        match self.state {
            State::Ride => {
                let clip = Self::ride_clip(s.speed);
                self.anim.keep(clip, 0.25);
                if s.crouch {
                    self.go(State::Crouch, "R_ANTIC_CROUCH_OLLIE_N_0_INTO", 0.12, 1.0);
                } else if s.push && !s.fakie && on_ground {
                    let clip = if s.speed > 4.0 { "R_PUSHHSP_HSTR_N_0_INTO" } else { "R_PUSHLSP_HSTR_N_0_INTO" };
                    self.go(State::PushInto, clip, 0.15, 1.0);
                } else if s.slide_side != 0.0 && s.speed > 2.0 {
                    self.go(State::SlideInto, "R_SLIDE_BS_HSP_INTO", 0.12, 1.4);
                } else if s.brake && s.speed > 0.3 {
                    self.go(State::BrakeInto, "R_BRAKE_N_N_0_INTO", 0.15, 1.0);
                }
            }
            State::PushInto | State::PushStroke => {
                a.pushing = true;
                if !self.pushed && self.anim.progress() >= PUSH_AT {
                    self.pushed = true;
                    a.push = true;
                }
                if self.anim.done() {
                    if s.push && s.speed < 8.4 {
                        let clip = if s.speed > 4.0 { "R_PUSHHSP_HSTR_N_0_INTO" } else { "R_PUSHLSP_HSTR_N_0_INTO" };
                        // Each further stroke from the push's own start, sped up a little: Skate 3's strokes come quicker.
                        self.go(State::PushStroke, clip, 0.12, 1.25);
                    } else {
                        let clip = if s.speed > 4.0 { "R_PUSH_H_N_OUT_FRONT" } else { "R_PUSH_L_N_OUT_FRONT" };
                        self.go(State::PushOut, clip, 0.1, 1.0);
                    }
                }
                if s.crouch {
                    self.go(State::Crouch, "R_ANTIC_CROUCH_OLLIE_N_0_INTO", 0.12, 1.0);
                }
            }
            State::PushOut | State::BrakeOut | State::SlideOut | State::Land => {
                if self.anim.done() {
                    self.state = State::Ride;
                    self.anim.play(Self::ride_clip(s.speed), 0.2, 1.0);
                } else if s.crouch {
                    self.go(State::Crouch, "R_ANTIC_CROUCH_OLLIE_N_0_INTO", 0.12, 1.0);
                }
            }
            State::BrakeInto | State::Brake => {
                a.brake = true;
                if self.state == State::BrakeInto && self.anim.done() {
                    self.go(State::Brake, "R_BRAKE_N_N_0_CYC", 0.1, 1.0);
                }
                if !s.brake || s.speed < 0.2 {
                    self.go(State::BrakeOut, "R_BRAKE_N_N_0_OUT", 0.1, 1.0);
                }
            }
            State::SlideInto | State::Slide => {
                a.slide = s.slide_side;
                if self.state == State::SlideInto && self.anim.done() {
                    self.go(State::Slide, "R_SLIDE_BS_HSP_CYC", 0.1, 1.0);
                }
                if s.slide_side == 0.0 || s.speed < 1.0 {
                    a.slide = 0.0;
                    self.go(State::SlideOut, "R_SLIDE_BS_HSP_OUT_000", 0.1, 1.3);
                }
            }
            State::Crouch => {
                if self.anim.done() && self.anim.playing() != "R_ANTIC_OLLIE_N_0_CYC" {
                    self.anim.play("R_ANTIC_OLLIE_N_0_CYC", 0.1, 1.0);
                }
                if !s.crouch {
                    self.state = State::Ride;
                    self.anim.play(Self::ride_clip(s.speed), 0.2, 1.0);
                }
            }
            State::Pop(trick) => {
                if self.anim.done() {
                    match trick_clips(trick) {
                        Some(tc) => {
                            let len = self.anim.index().clips.get(tc.cyc).map(|c| c.seconds()).unwrap_or(tc.cyc_seconds);
                            self.go(State::TrickTurn(trick), tc.cyc, 0.04, len / tc.cyc_seconds);
                        }
                        None => {
                            let high = self.anim.playing() == "OLLIE_HIGH_G";
                            self.go(State::Air, if high { "OLLIE_HIGH_A" } else { "OLLIE_LOW_A" }, 0.05, 1.0);
                        }
                    }
                }
            }
            State::TrickTurn(trick) => {
                if self.anim.progress() >= 1.0 {
                    if let Some(tc) = trick_clips(trick) {
                        self.go(State::TrickCatch(trick), tc.out, 0.04, 1.0);
                    }
                }
            }
            State::TrickCatch(_) => {
                if self.anim.progress() >= CAUGHT_AT {
                    self.caught = true;
                }
                if self.anim.done() {
                    self.go(State::Air, "IA_IDLE_N_N_0_CYC", 0.2, 1.0);
                }
            }
            State::Air => {
                if self.anim.done() && self.anim.playing() != "IA_IDLE_N_N_0_CYC" {
                    self.anim.play("IA_IDLE_N_N_0_CYC", 0.2, 1.0);
                }
            }
            State::Bail | State::GetUp => {}
        }
        // Rolled off a ledge without popping: in the air.
        if s.phase == Phase::Air && matches!(self.state, State::Ride | State::PushInto | State::PushStroke | State::PushOut | State::BrakeInto | State::Brake | State::BrakeOut | State::SlideInto | State::Slide | State::SlideOut | State::Crouch | State::Land) {
            self.caught = true;
            self.go(State::Air, "IA_IDLE_N_N_0_CYC", 0.2, 1.0);
        }
        self.anim.advance(dt);
        a
    }

    /// After the board's tick: landings and bails.
    pub fn after(&mut self, phase: Phase, landed: Option<f32>, bailed: bool, bail_time: f32, speed: f32) -> AfterTick {
        let mut out = AfterTick::default();
        if bailed && !matches!(self.state, State::Bail | State::GetUp) {
            self.go(State::Bail, "W_FLAIL_LEAN000TWIST000_N_0_N", 0.1, 1.0);
            out.bailed = true;
            return out;
        }
        if let Some(down) = landed {
            if phase == Phase::Bail {
                return out;
            }
            if !self.caught {
                // Landed before the trick was caught.
                self.go(State::Bail, "W_FLAIL_LEAN000TWIST000_N_0_N", 0.1, 1.0);
                out.bailed = true;
                out.bail_board = true;
                if let State::TrickTurn(t) | State::TrickCatch(t) | State::Pop(t) = self.state {
                    self.last_trick = Some((t, false));
                }
                return out;
            }
            let clip = if down > 5.0 { "L_HCOM_HIMP" } else if down > 3.0 { "L_HCOM_LIMP" } else { "L_LCOM" };
            if let State::TrickCatch(t) | State::TrickTurn(t) | State::Pop(t) = self.state {
                self.last_trick = Some((t, true));
                out.landed_trick = Some(t);
            }
            self.go(State::Land, clip, 0.08, 1.2);
            out.landed = true;
        }
        if self.state == State::Bail && phase == Phase::Bail && bail_time > 1.0 && speed < 1.0 {
            self.go(State::GetUp, "W_RECOVERY_ONFRONT_N_0_N", 0.25, 1.2);
        }
        if self.state == State::GetUp && self.anim.done() {
            out.recover = true;
            self.state = State::Ride;
            self.caught = true;
            self.anim.play(Self::ride_clip(0.0), 0.3, 1.0);
        }
        out
    }

    /// The share of the carve lean to lay over the clip: while riding, not mid-push.
    pub fn carve_share(&self) -> f32 {
        match self.state {
            State::Ride => 0.85,
            State::PushOut | State::Land => 0.4,
            _ => 0.0,
        }
    }
}

/// What a tick's end brought.
#[derive(Default, Debug)]
pub struct AfterTick {
    pub landed: bool,
    pub landed_trick: Option<Trick>,
    pub bailed: bool,
    /// The board must bail too (a trick not caught: the physics didn't see it).
    pub bail_board: bool,
    pub recover: bool,
}

/// The hips' height over the deck in a pose (for the pop's "how high is the body now").
pub fn hips_height(pose: &[Sample], parents: &[i32]) -> f32 {
    // HIPS is bone 1, under the detached trajectory: its translation is in model space.
    let _ = parents;
    pose.get(1).map(|s| Vec3::new(s[7], s[8], s[9]).y).unwrap_or(0.9)
}
