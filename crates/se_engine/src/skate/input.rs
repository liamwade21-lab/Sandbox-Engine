//! The skater's controls: one pad, filled from a gamepad (Skate 3's own layout and its "flick-it" right stick) or from the
//! keyboard and mouse (the mouse is the right stick: it springs back to the middle by itself), whichever was used last.
//!
//! - **Gamepad**: left stick steers (and turns in the air), A pushes (hold to keep pushing), B brakes, B with the stick to one
//!   side powerslides, the right stick does the tricks (pull it down to crouch, flick it up to pop: straight up an ollie, up and
//!   to the left a kickflip, up and to the right a heelflip, to a side a shove-it, round through a side and up a 360 flip or a
//!   laser flip), Y gets off the board.
//! - **Keyboard and mouse**: A and D steer, W pushes (held: keeps pushing), S brakes, Shift with A or D powerslides, the mouse
//!   is the right stick (pull it back, flick it forward), Space is an ollie (held: crouch, let go: pop), and 1-6 pop the
//!   tricks straight away (kickflip, heelflip, shove-it, frontside shove-it, 360 flip, laser flip), F gets off the board.
//!
//! Tricks are recognised from the stick's path by `Flick` (the same for both): a crouch (the stick held down) and then the
//! direction it is flicked to.

use bevy::math::Vec2;

/// A trick the right stick (or a key) asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trick {
    Ollie,
    Kickflip,
    Heelflip,
    Shuvit,
    FsShuvit,
    Flip360,
    Laser,
}

impl Trick {
    pub const ALL: [Trick; 7] = [Trick::Ollie, Trick::Kickflip, Trick::Heelflip, Trick::Shuvit, Trick::FsShuvit, Trick::Flip360, Trick::Laser];

    pub fn name(self) -> &'static str {
        match self {
            Trick::Ollie => "Ollie",
            Trick::Kickflip => "Kickflip",
            Trick::Heelflip => "Heelflip",
            Trick::Shuvit => "Pop shove-it",
            Trick::FsShuvit => "Frontside shove-it",
            Trick::Flip360 => "360 flip",
            Trick::Laser => "Laser flip",
        }
    }

    pub fn by_name(s: &str) -> Option<Trick> {
        let s = s.to_ascii_lowercase().replace(['-', ' ', '_'], "");
        Self::ALL.into_iter().find(|t| t.name().to_ascii_lowercase().replace(['-', ' ', '_'], "") == s || format!("{t:?}").to_ascii_lowercase() == s)
    }
}

/// This frame's controls, whatever they came from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    /// Left stick: x right, y forward (-1..1).
    pub left: Vec2,
    /// Right stick: x right, y up (-1..1).
    pub right: Vec2,
    /// Push held.
    pub push: bool,
    /// Brake held.
    pub brake: bool,
    /// Powerslide held (with a side on the left stick).
    pub slide: bool,
    /// Off the board (pressed this frame).
    pub off: bool,
    /// A trick asked for straight away (keys 1-6, a test's `skate trick`).
    pub trick: Option<Trick>,
    /// Space: held is a crouch, let go an ollie (on its own, not through the stick).
    pub crouch_key: bool,
    /// Something was pressed or moved this frame (Skate 3's no-input friction starts after a while without).
    pub any: bool,
}

/// What the stick's path asks for this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlickOut {
    /// Crouched: the stick is held down (or Space).
    pub crouch: bool,
    /// How long it has been, seconds (the pop's strength grows with it, as in Skate 3).
    pub crouched_for: f32,
    /// A pop this frame.
    pub pop: Option<Trick>,
}

/// The flick-it recogniser.
#[derive(Clone, Debug, Default)]
pub struct Flick {
    crouched: bool,
    crouched_for: f32,
    /// Since the stick left the bottom: how long, and whether it went round through a side (-1 left, 1 right).
    since_down: Option<f32>,
    via: f32,
    /// Flicked to this side (-1 left, 1 right): a shove-it if it comes back, a 360 or laser flip if it goes on up.
    side: f32,
    key_crouch: bool,
    key_for: f32,
}

/// The stick counts as down below this, and as flicked beyond this far out.
const DOWN: f32 = -0.55;
const RIM: f32 = 0.7;
/// A flick must reach the rim this soon after leaving the bottom.
const FLICK_S: f32 = 0.3;

impl Flick {
    pub fn step(&mut self, pad: &Pad, dt: f32) -> FlickOut {
        let s = pad.right;
        let mut out = FlickOut::default();
        // Space: a crouch while held, an ollie when let go.
        if pad.crouch_key {
            self.key_for = if self.key_crouch { self.key_for + dt } else { 0.0 };
            self.key_crouch = true;
        } else if self.key_crouch {
            self.key_crouch = false;
            out.pop = Some(Trick::Ollie);
            out.crouched_for = self.key_for;
            return out;
        }
        if s.y < DOWN && s.x.abs() < 0.6 {
            self.crouched_for = if self.crouched { self.crouched_for + dt } else { 0.0 };
            self.crouched = true;
            self.since_down = None;
            self.via = 0.0;
            self.side = 0.0;
        } else if self.crouched {
            // Leaving the bottom: the flick is on its way.
            self.crouched = false;
            self.since_down = Some(0.0);
            self.via = 0.0;
            self.side = 0.0;
        }
        if let Some(t) = self.since_down.as_mut() {
            *t += dt;
            if s.x.abs() > 0.65 && s.y < 0.25 {
                self.via = s.x.signum();
            }
            let angle = s.x.atan2(s.y).to_degrees(); // 0 up, + right
            let mut trick = None;
            if s.length() > RIM && s.y > -0.25 {
                if self.via != 0.0 && s.y > 0.3 && angle.signum() == self.via {
                    // Round through a side and on up: a 360 flip (left) or a laser flip (right).
                    trick = Some(if self.via < 0.0 { Trick::Flip360 } else { Trick::Laser });
                } else if angle.abs() < 25.0 {
                    trick = Some(Trick::Ollie);
                } else if angle.abs() < 68.0 {
                    trick = Some(if angle < 0.0 { Trick::Kickflip } else { Trick::Heelflip });
                } else {
                    // To a side: a shove-it, unless it goes on round and up (decided when it comes back or goes up).
                    self.side = angle.signum();
                }
            }
            if trick.is_none() && self.side != 0.0 && (s.length() < 0.45 || *t > FLICK_S + 0.15) {
                trick = Some(if self.side < 0.0 { Trick::Shuvit } else { Trick::FsShuvit });
            }
            if let Some(trick) = trick {
                out.pop = Some(trick);
                out.crouched_for = self.crouched_for;
                self.since_down = None;
                self.crouched_for = 0.0;
                self.side = 0.0;
            } else if *t > FLICK_S + 0.15 {
                self.since_down = None;
                self.crouched_for = 0.0;
                self.side = 0.0;
            }
        }
        if let Some(t) = pad.trick {
            out.pop = Some(t);
            out.crouched_for = 0.4;
        }
        out.crouch = self.crouched || self.key_crouch;
        out.crouched_for = if out.pop.is_some() { out.crouched_for } else if self.key_crouch { self.key_for } else { self.crouched_for };
        out
    }
}

/// The mouse as a stick: counts move it, and it springs back to the middle (about a tenth of a second).
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseStick {
    pub at: Vec2,
}

impl MouseStick {
    /// Counts a full throw takes.
    const THROW: f32 = 60.0;

    pub fn step(&mut self, counts: Vec2, dt: f32) -> Vec2 {
        // (Screen y grows downwards: pulling the mouse back is the stick down.)
        self.at += Vec2::new(counts.x, -counts.y) / Self::THROW;
        if self.at.length() > 1.0 {
            self.at = self.at.normalize();
        }
        let keep = (-dt / 0.09).exp();
        let out = self.at;
        self.at *= keep;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut Flick, path: &[(f32, f32)]) -> Option<Trick> {
        let mut got = None;
        for &(x, y) in path {
            let pad = Pad { right: Vec2::new(x, y), ..Default::default() };
            if let Some(t) = f.step(&pad, 1.0 / 60.0).pop {
                got = Some(t);
            }
        }
        got
    }

    #[test]
    fn flicks_are_the_tricks_skate_3_gives_them() {
        let down: Vec<(f32, f32)> = vec![(0.0, -1.0); 10];
        let with = |end: &[(f32, f32)]| down.iter().chain(end).copied().collect::<Vec<_>>();
        assert_eq!(run(&mut Flick::default(), &with(&[(0.0, 0.0), (0.0, 1.0)])), Some(Trick::Ollie));
        assert_eq!(run(&mut Flick::default(), &with(&[(-0.3, 0.0), (-0.7, 0.7)])), Some(Trick::Kickflip));
        assert_eq!(run(&mut Flick::default(), &with(&[(0.3, 0.0), (0.7, 0.7)])), Some(Trick::Heelflip));
        assert_eq!(run(&mut Flick::default(), &with(&[(-0.6, -0.5), (-1.0, 0.0), (-0.3, 0.0)])), Some(Trick::Shuvit));
        assert_eq!(run(&mut Flick::default(), &with(&[(0.6, -0.5), (1.0, 0.0), (0.3, 0.0)])), Some(Trick::FsShuvit));
        assert_eq!(run(&mut Flick::default(), &with(&[(1.0, -0.4), (1.0, 0.0), (0.6, 0.8)])), Some(Trick::Laser));
        assert_eq!(run(&mut Flick::default(), &with(&[(-1.0, -0.2), (-0.9, 0.2), (-0.5, 0.85)])), Some(Trick::Flip360));
        // No crouch first: nothing.
        assert_eq!(run(&mut Flick::default(), &[(0.0, 0.0), (0.0, 1.0)]), None);
        // Too slow from the bottom to the rim: nothing.
        let mut slow = down.clone();
        slow.extend(std::iter::repeat_n((0.0, 0.2), 30));
        slow.push((0.0, 1.0));
        assert_eq!(run(&mut Flick::default(), &slow), None);
        // Space: a crouch while held, an ollie when let go.
        let mut f = Flick::default();
        let held = Pad { crouch_key: true, ..Default::default() };
        assert!(f.step(&held, 0.1).crouch);
        assert_eq!(f.step(&Pad::default(), 0.1).pop, Some(Trick::Ollie));
        assert_eq!(Trick::by_name("kickflip"), Some(Trick::Kickflip));
        assert_eq!(Trick::by_name("360 flip"), Some(Trick::Flip360));
    }

    #[test]
    fn the_mouse_stick_springs_back() {
        let mut m = MouseStick::default();
        let v = m.step(Vec2::new(0.0, 90.0), 1.0 / 60.0);
        assert!(v.y < -0.99);
        for _ in 0..30 {
            m.step(Vec2::ZERO, 1.0 / 60.0);
        }
        assert!(m.at.length() < 0.01);
    }
}
