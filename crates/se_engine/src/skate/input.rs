//! The controls, as the Skate 3 Rust Engine reads them: one Xbox 360 controller's raw state a tick (`skate_sim::RawPad`), which
//! its own controller code, gesture recognisers (Skate 3's `.pat` files) and action graph then turn into what he does. A
//! gamepad is passed through as it is (Skate 3's layout); the keyboard and mouse make a controller's state: the mouse is the
//! right stick (its flicks are the tricks), keys stand for the buttons and the left stick, and a few keys play whole flicks.

use std::collections::VecDeque;

use bevy::math::Vec2;
use skate_sim::RawPad;

/// XInput's button bits (the order the engine reads).
pub mod buttons {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_STICK: u16 = 0x0040;
    pub const RIGHT_STICK: u16 = 0x0080;
    pub const LB: u16 = 0x0100;
    pub const RB: u16 = 0x0200;
    pub const A: u16 = 0x1000;
    pub const B: u16 = 0x2000;
    pub const X: u16 = 0x4000;
    pub const Y: u16 = 0x8000;
}

/// A flick played by a key, tick by tick: the right stick pulled down (the crouch), then moved through the sectors Skate 3's
/// recognisers read as the trick. The names are what they give in his stance (regular), found by running each path through
/// the simulation (`every_flick_key_gives_its_trick`): a path that jumps across sectors is read as nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flick {
    Ollie,
    Kickflip,
    Heelflip,
    FsShuvit,
    Shuvit,
    VarialKickflip,
    VarialHeelflip,
    Hardflip,
    InwardHeelflip,
}

/// The stick held in a compass sector (0 up, 1 up-right, 2 right, ... 7 up-left).
fn sector(k: usize) -> [f32; 2] {
    let a = (90.0 - 45.0 * k as f32).to_radians();
    [a.cos(), a.sin()]
}

impl Flick {
    pub const ALL: [Flick; 9] = [
        Flick::Ollie,
        Flick::Kickflip,
        Flick::Heelflip,
        Flick::FsShuvit,
        Flick::Shuvit,
        Flick::VarialKickflip,
        Flick::VarialHeelflip,
        Flick::Hardflip,
        Flick::InwardHeelflip,
    ];

    /// The trick's name as the engine's HUD gives it.
    pub fn name(self) -> &'static str {
        match self {
            Flick::Ollie => "Ollie",
            Flick::Kickflip => "Kickflip",
            Flick::Heelflip => "Heelflip",
            Flick::FsShuvit => "FS Pop Shuvit",
            Flick::Shuvit => "Pop Shuvit",
            Flick::VarialKickflip => "Varial Kickflip",
            Flick::VarialHeelflip => "Varial Heelflip",
            Flick::Hardflip => "Hardflip",
            Flick::InwardHeelflip => "Inward Heelflip",
        }
    }

    pub fn by_name(s: &str) -> Option<Flick> {
        let want = s.to_ascii_lowercase().replace([' ', '_', '-'], "");
        Flick::ALL.into_iter().find(|f| f.name().to_ascii_lowercase().replace(' ', "") == want || format!("{f:?}").to_ascii_lowercase() == want)
    }

    /// The right stick a tick at a time (60 a second), from the crouch to the stick let go.
    pub fn samples(self) -> Vec<[f32; 2]> {
        let mut s = vec![[0.0, -1.0]; 18];
        let two = |a: usize, b: usize| [vec![sector(a); 3], vec![sector(b); 3]].concat();
        let tail: Vec<[f32; 2]> = match self {
            Flick::Ollie => vec![sector(0); 5],
            Flick::Kickflip => two(1, 0),
            Flick::Heelflip => vec![sector(7); 5],
            Flick::FsShuvit => two(5, 6),
            // Round through the right to the top.
            Flick::Shuvit => [[-30f32, 30.0, 90.0].iter().map(|d| [d.to_radians().cos(), d.to_radians().sin()]).collect(), vec![sector(0); 6]].concat(),
            Flick::VarialKickflip => two(3, 0),
            Flick::VarialHeelflip => two(5, 0),
            Flick::Hardflip => two(5, 1),
            Flick::InwardHeelflip => two(3, 7),
        };
        s.extend(tail);
        s.push([0.0, 0.0]);
        s
    }
}

/// The mouse as a stick: its motion pushes the stick, which springs back to the middle when the mouse rests (about 0.09 s),
/// so a quick pull back and push forward is a flick, as a thumb on the stick makes one.
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseStick {
    pub at: Vec2,
}

/// Mouse counts for a full stick.
const MOUSE_FULL: f32 = 60.0;

impl MouseStick {
    /// This frame's mouse motion (counts, y down as the mouse reports it) into the stick (y up).
    pub fn step(&mut self, counts: Vec2, dt: f32) -> Vec2 {
        let back = (-dt / 0.03).exp();
        self.at = self.at * back + Vec2::new(counts.x, -counts.y) / MOUSE_FULL;
        if self.at.length() > 1.0 {
            self.at = self.at.normalize();
        }
        if self.at.length() < 0.05 {
            self.at = Vec2::ZERO;
        }
        self.at
    }
}

/// Which device's controls count: the one used last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Device {
    #[default]
    Keyboard,
    Pad(bevy::prelude::Entity),
}

/// What the frame's controls hold, and the flicks queued tick by tick.
#[derive(Clone, Debug, Default)]
pub struct Controls {
    pub device: Device,
    /// The controller as the frame's devices make it (sticks, triggers, buttons).
    pub pad: RawPad,
    /// Right-stick samples a key's flick still has to play (one a tick, over the pad's own).
    pub queue: VecDeque<[f32; 2]>,
    /// Space held: the stick held down (the crouch); let go, it flicks up for a few ticks.
    pub space_held: bool,
    pub mouse: MouseStick,
}

impl Controls {
    /// A queued flick's next sample over the frame's controller, for one tick (`None` when no flick plays).
    pub fn tick_pad(&mut self) -> Option<RawPad> {
        let r = self.queue.pop_front()?;
        Some(RawPad { right_stick: r, ..self.pad })
    }

    pub fn play(&mut self, flick: Flick) {
        self.queue = flick.samples().into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flick_starts_with_the_crouch_and_ends_let_go() {
        for f in Flick::ALL {
            let s = f.samples();
            assert_eq!(s[0], [0.0, -1.0]);
            assert_eq!(*s.last().unwrap(), [0.0, 0.0]);
            assert_eq!(Flick::by_name(f.name()), Some(f));
        }
        let mut c = Controls::default();
        c.pad.right_stick = [0.3, 0.3];
        c.play(Flick::Ollie);
        assert_eq!(c.tick_pad().unwrap().right_stick, [0.0, -1.0]);
        c.queue.clear();
        assert_eq!(c.tick_pad(), None);
    }

    /// Each flick key, played after pushing to speed on a flat floor, is the trick it's named for (the mod's own files).
    #[test]
    #[ignore = "runs the simulation: cargo test -p se_engine --release every_flick_key -- --ignored"]
    fn every_flick_key_gives_its_trick() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/skate3");
        let h = 400.0;
        for f in Flick::ALL {
            let mut sim = skate_sim::Sim::load(&root, "easy").unwrap();
            sim.set_world(&[[-h, 0.0, -h], [h, 0.0, -h], [h, 0.0, h], [-h, 0.0, h]], &[[0, 3, 2], [0, 2, 1]]).unwrap();
            sim.spawn([0.0, 0.0, 0.0], 0.0).unwrap();
            for t in 0..180 {
                sim.step(&RawPad { buttons: if t < 150 { buttons::A } else { 0 }, ..Default::default() }).unwrap();
            }
            let path = f.samples();
            for t in 0..150 {
                sim.step(&RawPad { right_stick: path.get(t).copied().unwrap_or([0.0, 0.0]), ..Default::default() }).unwrap();
            }
            assert_eq!(sim.hud().landed_trick_text, f.name(), "{f:?}");
        }
    }

    #[test]
    fn the_mouse_stick_springs_back() {
        let mut m = MouseStick::default();
        assert!(m.step(Vec2::new(0.0, 80.0), 1.0 / 60.0).y < -0.9);
        for _ in 0..10 {
            m.step(Vec2::ZERO, 1.0 / 60.0);
        }
        assert_eq!(m.at, Vec2::ZERO);
    }
}
