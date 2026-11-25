//! Getting in and out of the Sidewinder and sitting in it (the spec's F2, step 1.3): E at the parked ship boards it, the camera
//! sits at the pilot's eyes with Elite's head look on the middle button, and B gets him out: landed or within reach of the
//! ground, beside it; anywhere else the ship is gone and he is in the free flight where it was (ARCHITECTURE.md A12, the
//! user's choice over the spec's D8).
//! Both cuts, into the seat and out of it, come up from black over `FADE_S` real seconds (ours, F: Elite's own
//! boarding is Odyssey's walk-in sequence, not built).
//!
//! - Boarding turns the mod on (`ModControl::board`): the sandbox leaves first person at the end of that frame and the seat
//!   writes the camera from then on (`seat_camera`, bound to the body every frame: the 500 m-a-frame rule).
//! - Seated, every key's press is taken from the frame's input (`take_keys`) once Elite's bindings have been read, so the
//!   port's own readers never see them (K would slow the clock, I open the wiki); Esc (the menu) and F11 (a screenshot) are
//!   left. The game clock runs at real time while he sits and is given back when he gets out (the spec's edge cases).
//! - Getting out (`leave_rule`): B (or the role panel's Disembark) anywhere; landed or within reach of the ground
//!   (`within_step_out`) he is out beside the ship, whichever way he leaves; out of reach, B abandons the ship (A12) and the
//!   mod turned off by any other way loses it (the spec's edge cases: "if that was in flight it is lost"). Every way out (B,
//!   the mod off, the ship destroyed, lost in hyperspace, removed) goes through one function, `release` (`exit_rule` says
//!   where he goes, the spec's F6), which writes the camera where the seat's camera is (`seat_at`, `write_seat_camera`),
//!   turned as the ship without the head's turn; he goes beside the ship's left side when it stays (landed or within
//!   reach); in the free flight where the ship was when he abandoned it; on the ground under it when it was lost flying by
//!   ground he can stand on, else on Earth, unless something else already put him somewhere (the main menu's Play puts him
//!   on Earth). The seat's view stays until the sandbox has set him down (in the free flight: not past the release's frame).

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{DQuat, DVec3, Quat, Vec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::egui;
use se_core::elite_asset::Binds;

use crate::camera::FlyCamera;
use crate::mods::{ModControl, ModRegistry};
use crate::sandbox::{Mode, Sandbox};
use crate::system::SystemBody;
use crate::terrain::Terrain;
use crate::universe::SimTime;

use super::panels::{Panels, Side};
use super::pilot::{Pilot, PilotKeys};
use super::{Sidewinder, SidewinderAssets, CODE};

/// On foot, the ship is boarded from within this far of its hull while looking at it (the spec's F2).
pub const BOARD_REACH_M: f32 = 6.0;
/// Head look: radians a mouse count, the most to each side and up or down (Elite's head look stops near the canopy's edges:
/// E, to be matched to the user's pictures).
const LOOK_RATE: f32 = 0.0022;
const LOOK_YAW_MOST: f32 = 2.0;
const LOOK_PITCH_MOST: f32 = 1.1;
/// Out of the seat: this far out from the hull's left side, on the ground.
const EXIT_OUT_M: f32 = 2.5;
/// The fade from black after boarding and after getting out (real seconds; ours, F: kept short). A longer frame than
/// `FADE_MOST_DT` counts as that much, so a hitch while the view changes doesn't swallow it.
pub const FADE_S: f32 = 0.5;
const FADE_MOST_DT: f32 = 0.1;

/// The seat's state while he sits.
#[derive(Resource, Default)]
pub struct Seat {
    pub seated: bool,
    /// The camera is at the pilot's eye this frame (`seat_camera` wrote it, in hyperspace's tunnel too): what `draw` shows
    /// from (the seat's set) and when the view takes Elite's field of view (`seat_fov`).
    pub view: bool,
    /// Head look: on, and the view's turn from straight ahead (radians: yaw to the left, pitch up).
    pub head_look: bool,
    pub look: Vec2,
    /// The head look camera this frame (`head_aim`): its turn and its field of view's channel. Head look's turn while it is
    /// on, else the open side panel's (with that panel's field of view), else straight ahead; a test's `sw look` over all.
    /// `seat_camera` writes it, the camera and the projection (`seat_fov`) take it and the HUD draws with it (after
    /// `seat_camera`: the mod's `plugin` orders them).
    pub head: Head,
    /// A test's head turn (`sw look YAW PITCH`; None: the head is the pilot's again). Taken as it is, with no limits (a yaw
    /// past 180 degrees comes round the other way, a pitch past 90 looks on over backwards, upside down), but only if
    /// finite: `seat_camera` drops one that is not (`test_turn`).
    pub test_look: Option<Vec2>,
    /// Elite's FSS screen covers the view (`ui::screens`, `SE_SW_UI=1`, written every seated frame): the head straight ahead,
    /// head look not toggled, the mouse not steering (`pilot::fly`).
    pub screen: bool,
    /// B was pressed this frame (`seat_camera` acts on it).
    leave: bool,
    /// The ship is to be removed while he sits in it (`ShipRequest::Remove`: `seat_camera` releases the seat, then takes it).
    remove: bool,
    regain: u8,
    /// The game clock's rate and pause as they were when he sat down (given back when he gets out).
    clock: Option<(f64, bool)>,
    /// The fade from black after boarding or getting out: real seconds left of it (0: none).
    pub fade: f32,
    /// Out of the seat and not yet set down by the sandbox: the seat's view is kept meanwhile (`seat_camera`).
    setting_down: bool,
    /// Boarded and the sandbox not yet in the seat's mode (`Mode::Pilot`): only then is a boarding from anywhere but on foot
    /// refused (`boarding_refused`).
    boarding: bool,
    /// The keys and buttons that were down when he sat down (E that boarded, above all): none of them counts for the ship
    /// until it has been let go once (`unheld`; E is the ship's thrust to the right).
    held_keys: Vec<KeyCode>,
    held_buttons: Vec<MouseButton>,
}

impl Seat {
    /// Just sat down: the game clock to real time and running, its old state kept; what is held down is the boarding's.
    pub fn boarded(sim: &mut SimTime, keys: &ButtonInput<KeyCode>, buttons: &ButtonInput<MouseButton>) -> Seat {
        let clock = Some((sim.rate, sim.paused));
        sim.rate = 1.0;
        sim.paused = false;
        let (held_keys, held_buttons) = (keys.get_pressed().copied().collect(), buttons.get_pressed().copied().collect());
        Seat { seated: true, regain: 3, clock, fade: FADE_S, boarding: true, held_keys, held_buttons, ..default() }
    }

    /// Get out (as B does: anywhere, `leave_rule`).
    pub fn request_leave(&mut self) {
        self.leave = true;
    }

    /// The ship is to be removed while he sits in it: the seat is released first (`seat_camera`).
    pub fn request_remove(&mut self) {
        self.remove = true;
    }

    /// The fade's black now (0 clear .. 1 black).
    pub fn fade_alpha(&self) -> f32 {
        (self.fade / FADE_S).clamp(0.0, 1.0)
    }
}

/// The fade's seconds left after a frame of `real_dt` real seconds.
pub fn fade_step(left: f32, real_dt: f32) -> f32 {
    let left = left - real_dt.min(FADE_MOST_DT);
    // (Frames' f32 seconds add up a hair off the whole: within a tenth of a millisecond it is over.)
    if left > 1e-4 { left } else { 0.0 }
}

/// The fade's clock, on real seconds (`timing frame` steps them too, so a test's frames are its seconds); its end is logged
/// with the frames and seconds it took.
pub fn fade_clock(time: Res<Time<Real>>, mut seat: ResMut<Seat>, mut took: Local<(u32, f32)>) {
    if seat.fade <= 0.0 {
        return;
    }
    let dt = time.delta_secs();
    seat.fade = fade_step(seat.fade, dt);
    took.0 += 1;
    took.1 += dt;
    if seat.fade == 0.0 {
        info!("sidewinder: the fade is over ({} frames, {:.3} real s)", took.0, took.1);
        *took = (0, 0.0);
    }
}

//// What getting out does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    /// Out beside the ship, which stays parked (landed or within reach of the ground, whichever way he leaves).
    Out,
    /// Out into the free flight where the ship was, and the ship is gone (B or Disembark anywhere else: A12).
    Abandon,
    /// Out, and the ship is lost (the mod turned off another way, flying out of reach of the ground).
    Lost,
}

/// The rule for getting out (ARCHITECTURE.md A12, replacing the spec's D8; the spec's edge cases): `by_ground` for the ship
/// within reach to step out (`within_step_out`), `by_b` for B or Disembark, else the mod turned off by any way. Landed or
/// within reach, he is out beside it and the ship stays, whichever way he leaves (a ship on the ground is never deleted by
/// leaving it); anywhere else B abandons it and any other way loses it. Nothing refuses; the ship goes only where he cannot
/// be put on foot.
pub fn leave_rule(landed: bool, by_ground: bool, by_b: bool) -> Leave {
    match (landed || by_ground, by_b) {
        (true, _) => Leave::Out,
        (false, true) => Leave::Abandon,
        (false, false) => Leave::Lost,
    }
}

/// Why the seat is released (every way out of it goes through `release`), as `seat_camera` decided it: `exit_rule` takes
/// the decision as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Out beside the ship, which stays (`Leave::Out`): landed or within reach of the ground, by B, Disembark or the mod
    /// turned off by any way.
    Out,
    /// B or Disembark anywhere else (higher, over no ground he can stand on, the glide, supercruise, the jump's countdown,
    /// the tunnel): the ship is gone and he is in the free flight (`Leave::Abandon`, A12).
    Abandon,
    /// The mod turned off by any other way while not landed and out of reach of the ground (`Leave::Lost`): the ship is lost.
    Lost,
    /// The hull at zero or the oxygen run out.
    Destroyed,
    /// The ship removed (`sw remove`, the Mods row) while he sits in it.
    Removed,
}

impl From<Leave> for Why {
    fn from(leave: Leave) -> Why {
        match leave {
            Leave::Out => Why::Out,
            Leave::Abandon => Why::Abandon,
            Leave::Lost => Why::Lost,
        }
    }
}

/// Where he goes out of the seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Beside the ship's left side, on the ground (landed, or within reach of the ground: the ship is set down there).
    Beside,
    /// On the ground under the ship, on its body (it was flying: destroyed, lost or removed in flight).
    Under,
    /// On Earth, as the main menu's Play puts him: there is no ground to stand on by the ship.
    Earth,
    /// In the port's free flight where the ship was (`Mode::Fly`, what V gives): he abandoned it (A12). Nothing is set down.
    Flight,
    /// Nowhere: the sandbox isn't in the seat's mode (he is already somewhere: the mod went off before it took the camera).
    Stay,
}

/// The release's rule (the spec's F6): whether the ship is lost and where he goes. Out (`Why::Out`): the ship stays and he is
/// beside it. Abandoned (A12): the ship is lost and he is in the free flight where it was, whatever the ground. Lost,
/// destroyed or removed: beside the ship when it is landed, on the ground under it when it was flying by a body he can stand
/// on (`ground`: a solid body's loaded ground, out of supercruise and hyperspace); else (deep space, the tunnel, a star, a
/// gas giant, ground not loaded, no ship) on Earth. Outside the seat's mode he stays where he is.
pub fn exit_rule(why: Why, landed: bool, ground: bool, ship: bool, in_seat_mode: bool) -> (bool, Place) {
    let lost = why != Why::Out;
    let place = match (in_seat_mode, why, ship && ground, landed || why == Why::Out) {
        (false, ..) => Place::Stay,
        (true, Why::Abandon, ..) => Place::Flight,
        (true, _, false, _) => Place::Earth,
        (true, _, true, true) => Place::Beside,
        (true, _, true, false) => Place::Under,
    };
    (lost, place)
}

/// Whether there is ground to stand on by the ship (`exit_rule`'s `ground`): a solid body's (not a star's or a gas giant's)
/// ground loaded under it (`level`: the node it comes from, None for no such ground), out of supercruise and hyperspace
/// (there the ship's body can be millions of km away, or a star with no ground at all). The sea counts, as it does for the
/// ship (the spec: every solid world is landable, the sea is ground for the ship, F): its surface is what `ship::ground_on`
/// gives over an ocean, and the sandbox stands him on it (`Sandbox::set_down`, its "(the sea)" case).
pub fn ground_by_ship(level: Option<u8>, in_cruise: bool, in_jump: bool) -> bool {
    level.is_some() && !in_cruise && !in_jump
}

/// Within reach to step out (A12: `leave_rule`'s `by_ground`): ground to stand on by the ship (`ground_by_ship`) and the
/// ship's lowest point no higher than `BOARD_REACH_M` over it (`height_m`: `GroundUnder::lowest_m`): the distance he boards
/// it from is the distance he steps out from.
pub fn within_step_out(level: Option<u8>, height_m: f64, in_cruise: bool, in_jump: bool) -> bool {
    ground_by_ship(level, in_cruise, in_jump) && height_m <= f64::from(BOARD_REACH_M)
}

/// Whether the free flight starts slow after an abandon (A12: its speed then starts at V's pace): ground to stand on by the
/// ship (`ground_by_ship`) within `SOLID_REACH_M` of it (`height_m`: the ship's origin over that loaded ground), the reach
/// within which the ship belongs to the sandbox's world. Higher, in supercruise or in hyperspace: no.
pub fn flight_by_ground(level: Option<u8>, height_m: f64, in_cruise: bool, in_jump: bool) -> bool {
    ground_by_ship(level, in_cruise, in_jump) && height_m <= super::SOLID_REACH_M
}

/// Where the seat's camera is this frame: the one decision `seat_camera` writes the camera by and `release` writes it once
/// more by, so the two never disagree. Hyperspace's tunnel first (the jump's pose in the universe, whatever bodies still
/// exist: the old system's live on for the tunnel's first frames while the target is generated), else the ship's place in
/// its body's frame.
enum SeatAt<'a> {
    Tunnel(super::hyper::Jump),
    Body { body: &'a SystemBody, terrain: Option<&'a Terrain>, pos: DVec3, rot: DQuat },
    Nowhere,
}

fn seat_at<'a>(pilot: &Pilot, ship: &Sidewinder, bodies: &'a Query<(&SystemBody, Option<&Terrain>)>) -> SeatAt<'a> {
    if let Some(j) = pilot.jump {
        return SeatAt::Tunnel(j);
    }
    let Some(p) = ship.parked.as_ref() else { return SeatAt::Nowhere };
    match bodies.iter().find(|(b, _)| b.obj == p.obj) {
        Some((body, terrain)) => SeatAt::Body { body, terrain, pos: p.pos, rot: p.rot },
        None => SeatAt::Nowhere,
    }
}

/// The ground by the seat's place (`seat_at`), the one reading `seat_camera`'s rule and `release` both use.
#[derive(Clone, Copy, Debug)]
struct GroundUnder {
    /// The loaded ground's node level under the ship on a solid body (`ship::ground_on` gives the sea's surface over an
    /// ocean, as the sandbox's set-down reads it); None with no such ground (a star, a gas giant, the tunnel, no body).
    level: Option<u8>,
    /// The ship's origin over that ground (its distance from the centre less the ground's), m: `flight_by_ground`'s.
    origin_m: f64,
    /// The lowest corner of the ship's model box over the ground under that corner (`pilot::over_ground`), m: about 0 for a
    /// ship lying on its side or back on the ground, its feet's or belly's height for a level one. `within_step_out`'s.
    lowest_m: f64,
}

fn ground_under(at: &SeatAt, shape: &super::ShipShape) -> GroundUnder {
    match at {
        SeatAt::Body { body: b, terrain: Some(t), pos, rot } if !b.star && !t.proc.kind.is_giant() => {
            let (ground_r, level) = crate::ship::ground_on(b.radius_m, b.shape, Some(*t), *pos);
            let lowest_m = box_corners(shape.lo, shape.hi).into_iter().map(|c| super::pilot::over_ground(b, Some(*t), 0.0, *pos, *rot, c)).fold(f64::INFINITY, f64::min);
            GroundUnder { level, origin_m: pos.length() - ground_r, lowest_m }
        }
        _ => GroundUnder { level: None, origin_m: f64::INFINITY, lowest_m: f64::INFINITY },
    }
}

/// The eight corners of a box from its least to its greatest corner (ship space).
fn box_corners(lo: Vec3, hi: Vec3) -> [Vec3; 8] {
    std::array::from_fn(|i| Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }))
}

/// The ship's level heading frame at `pos` (body frame, metres from the centre), as a turn from ship space (X right, Y up,
/// -Z the nose): up the radial there, forward the nose `rot` gives laid onto the horizontal plane, right from the two. With
/// the nose within `LEVEL_NOSE_MIN` (sine) of straight up or down, the belly (nose up) or the top (nose down) laid onto the
/// plane is the heading instead, so there always is one. Where he steps out of a ship that was not landed (`release`).
fn level_frame(pos: DVec3, rot: DQuat) -> DQuat {
    let up = pos.normalize_or(DVec3::Y);
    let flat = |v: DVec3| v - up * v.dot(up);
    let nose = rot * DVec3::NEG_Z;
    let mut forward = flat(nose);
    if forward.length() < LEVEL_NOSE_MIN {
        forward = flat(rot * if nose.dot(up) > 0.0 { DVec3::NEG_Y } else { DVec3::Y });
    }
    let forward = forward.normalize_or(up.any_orthonormal_vector());
    DQuat::from_mat3(&bevy::math::DMat3::from_cols(forward.cross(up), up, -forward)).normalize()
}

/// The turn he is handed at the release beside or under the ship (ship space to the body's frame: the camera's turn and the
/// pose `Sandbox::set_down` is given, and the frame he steps out by): the ship's own, except out of a ship that was not
/// landed (beside it, within reach of the ground), where it is its level heading frame (`level_frame`), so he faces its
/// heading level, never down the nose of a ship tipped or on its tail.
fn out_turn(place: Place, landed: bool, pos: DVec3, rot: DQuat) -> DQuat {
    match (place, landed) {
        (Place::Beside, false) => level_frame(pos, rot),
        _ => rot,
    }
}

/// `level_frame`'s nose is "straight up or down" within this sine of the vertical (about 3 degrees).
const LEVEL_NOSE_MIN: f64 = 0.05;

/// Whether a boarding is refused this frame (`seat_camera`): only while boarding, before the sandbox first gives the seat
/// its mode, and only from anywhere but on foot (the sandbox gives the seat its mode only from walking: a test's `sw board`
/// in the free flight would leave the free camera where it was). Once seated in the seat's mode, a mode the seat loses is
/// one of the ways out (`release`), never a refused boarding: a refusal there would free the seat with the ship left hanging
/// in the air.
pub fn boarding_refused(boarding: bool, mode: Mode) -> bool {
    boarding && !matches!(mode, Mode::Walk | Mode::Pilot)
}

/// The release's state changes (the pure part of `release`): out of the seat, the game clock given back, head look, a
/// test's head turn and an open side panel cleared (boarding again starts looking straight ahead), the fade from black, the
/// mod asked off if it is the one piloting, and the flight's transient states (hyperspace's tunnel, supercruise, the glide)
/// ended.
fn free_seat(seat: &mut Seat, sim: &mut SimTime, control: &mut ModControl, pilot: &mut Pilot, panels: &mut Panels) {
    seat.seated = false;
    seat.boarding = false;
    seat.leave = false;
    seat.remove = false;
    seat.head_look = false;
    seat.look = Vec2::ZERO;
    seat.test_look = None;
    seat.head = Head::default();
    panels.close();
    seat.fade = FADE_S;
    seat.setting_down = true;
    if let Some((rate, paused)) = seat.clock.take() {
        sim.rate = rate;
        sim.paused = paused;
    }
    if control.pilot {
        control.request_off();
    }
    pilot.jump = None;
    pilot.cruise = None;
    pilot.glide = None;
}

/// Elite's name for a key as Bevy's key code (the keys its default files use).
pub fn key_code(name: &str) -> Option<KeyCode> {
    use KeyCode as K;
    let k = name.strip_prefix("Key_")?;
    Some(match k {
        "A" => K::KeyA, "B" => K::KeyB, "C" => K::KeyC, "D" => K::KeyD, "E" => K::KeyE, "F" => K::KeyF, "G" => K::KeyG, "H" => K::KeyH,
        "I" => K::KeyI, "J" => K::KeyJ, "K" => K::KeyK, "L" => K::KeyL, "M" => K::KeyM, "N" => K::KeyN, "O" => K::KeyO, "P" => K::KeyP,
        "Q" => K::KeyQ, "R" => K::KeyR, "S" => K::KeyS, "T" => K::KeyT, "U" => K::KeyU, "V" => K::KeyV, "W" => K::KeyW, "X" => K::KeyX,
        "Y" => K::KeyY, "Z" => K::KeyZ,
        "0" => K::Digit0, "1" => K::Digit1, "2" => K::Digit2, "3" => K::Digit3, "4" => K::Digit4, "5" => K::Digit5, "6" => K::Digit6,
        "7" => K::Digit7, "8" => K::Digit8, "9" => K::Digit9,
        "Tab" => K::Tab, "Space" => K::Space, "Enter" => K::Enter, "Backspace" => K::Backspace, "Delete" => K::Delete, "Insert" => K::Insert,
        "Home" => K::Home, "End" => K::End, "PageUp" => K::PageUp, "PageDown" => K::PageDown,
        "UpArrow" => K::ArrowUp, "DownArrow" => K::ArrowDown, "LeftArrow" => K::ArrowLeft, "RightArrow" => K::ArrowRight,
        "LeftShift" => K::ShiftLeft, "RightShift" => K::ShiftRight, "LeftControl" => K::ControlLeft, "RightControl" => K::ControlRight,
        "LeftAlt" => K::AltLeft, "RightAlt" => K::AltRight, "Equals" => K::Equal, "Minus" => K::Minus, "Comma" => K::Comma, "Period" => K::Period,
        "Slash" => K::Slash, "backslash" | "BackSlash" => K::Backslash, "SemiColon" => K::Semicolon, "Apostrophe" => K::Quote,
        "LeftBracket" => K::BracketLeft, "RightBracket" => K::BracketRight, "Grave" => K::Backquote,
        "F1" => K::F1, "F2" => K::F2, "F3" => K::F3, "F4" => K::F4, "F5" => K::F5, "F6" => K::F6, "F7" => K::F7, "F8" => K::F8,
        "F9" => K::F9, "F10" => K::F10, "F11" => K::F11, "F12" => K::F12,
        "Numpad_0" => K::Numpad0, "Numpad_1" => K::Numpad1, "Numpad_2" => K::Numpad2, "Numpad_3" => K::Numpad3, "Numpad_4" => K::Numpad4,
        "Numpad_5" => K::Numpad5, "Numpad_6" => K::Numpad6, "Numpad_7" => K::Numpad7, "Numpad_8" => K::Numpad8, "Numpad_9" => K::Numpad9,
        _ => return None,
    })
}

/// Is a key down that the action is bound to (its primary or secondary keyboard key)?
pub fn action_pressed(b: &Binds, keys: &ButtonInput<KeyCode>, action: &str, just: bool) -> bool {
    b.get(action).is_some_and(|a| {
        a.keys().filter(|k| k.device == "Keyboard" && k.modifiers.is_empty()).filter_map(|k| key_code(&k.key)).any(|c| if just { keys.just_pressed(c) } else { keys.pressed(c) })
    })
}

/// The ray from his eyes (the sandbox's local frame) against the parked ship's box: the distance to it if it is hit.
fn look_at_ship(ship: &Sidewinder, sb: &Sandbox, assets: &SidewinderAssets) -> Option<f32> {
    let p = ship.parked.as_ref()?;
    let world = sb.world.as_ref().filter(|w| w.frame.obj == p.obj)?;
    let at = world.frame.from_body(p.pos).as_vec3();
    let rot = (world.frame.rotation().inverse() * p.rot).normalize().as_quat();
    let inv = rot.inverse();
    let (o, d) = (inv * (sb.eye - at), inv * sb.forward);
    let (lo, hi) = (assets.shape.lo, assets.shape.hi);
    let (mut t0, mut t1) = (0.0f32, f32::MAX);
    for k in 0..3 {
        if d[k].abs() < 1e-6 {
            if o[k] < lo[k] || o[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo[k] - o[k]) / d[k], (hi[k] - o[k]) / d[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    (t0 <= t1).then_some(t0)
}

/// On foot: E while looking at the ship within reach boards it (the press is taken, so "use" doesn't see it).
#[allow(clippy::too_many_arguments)]
pub fn board(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    sb: Res<Sandbox>,
    ship: Res<Sidewinder>,
    assets: Option<Res<SidewinderAssets>>,
    mut control: ResMut<ModControl>,
    reg: Res<ModRegistry>,
    mut seat: ResMut<Seat>,
    mut prompt: ResMut<BoardPrompt>,
    mut pilot: ResMut<Pilot>,
    mut sim: ResMut<SimTime>,
) {
    prompt.0 = false;
    let Some(assets) = assets else { return };
    // (While a gun or his hands hold a prop E is theirs: the physics gun turns the held prop with it, Garry's Mod's rule.)
    if !sb.walking() || !sb.placed || sb.menu || control.main_menu || control.skate || sb.skate.is_some() || sb.held.is_some() {
        return;
    }
    let Some(d) = look_at_ship(&ship, &sb, &assets) else { return };
    if d > BOARD_REACH_M {
        return;
    }
    prompt.0 = true;
    if keys.just_pressed(KeyCode::KeyE) {
        keys.clear_just_pressed(KeyCode::KeyE);
        if let Some(id) = reg.mods.iter().find(|m| m.code == CODE).map(|m| m.id.clone()) {
            control.board(&reg, &id);
            *seat = Seat::boarded(&mut sim, &keys, &buttons);
            // (Whether it stands landed or flies from where it hangs is the flight's to decide, at the velocity it had.)
            let vel = ship.parked.as_ref().map_or(DVec3::ZERO, |p| p.velocity(sb.world.as_ref()));
            *pilot = pilot.boarding(vel);
            info!("sidewinder: boarded ({d:.1} m from the hull)");
        }
    }
}

/// "Press E to board" while it would (drawn as Odyssey's prompt, near the middle: the look is E until the user's pictures).
#[derive(Resource, Default)]
pub struct BoardPrompt(pub bool);

pub fn draw_prompt(mut contexts: bevy_egui::EguiContexts, prompt: Res<BoardPrompt>, control: Res<ModControl>, seat: Res<Seat>) -> Result {
    // The fade from black, seated or not (over the HUD, which is drawn on the background layer).
    if seat.fade > 0.0 && !control.main_menu {
        let ctx = contexts.ctx_mut()?;
        let r = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("sidewinder fade"))).rect_filled(r, 0.0, egui::Color32::from_black_alpha((255.0 * seat.fade_alpha()) as u8));
    }
    if !prompt.0 || control.main_menu {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let r = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("sidewinder board prompt")));
    let at = egui::pos2(r.center().x, r.center().y + r.height() * 0.12);
    let text = "[E]  Board the Sidewinder";
    let font = egui::FontId::proportional(20.0);
    painter.text(at + egui::vec2(1.5, 1.5), egui::Align2::CENTER_CENTER, text, font.clone(), egui::Color32::from_black_alpha(180));
    painter.text(at, egui::Align2::CENTER_CENTER, text, font, egui::Color32::from_rgb(255, 140, 20));
    Ok(())
}

/// Seated: Elite's bound keys are taken from the frame's input (nothing else sees their presses), B to leave, the middle button
/// toggles head look, and the mouse turns the head while it's on. The cursor is locked as the walker's is.
#[allow(clippy::too_many_arguments)]
pub fn take_keys(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    assets: Option<Res<SidewinderAssets>>,
    control: Res<ModControl>,
    mut seat: ResMut<Seat>,
    window: Single<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    cfg: Res<crate::config::EngineConfig>,
    mut pilot_keys: ResMut<PilotKeys>,
    pads: Query<&bevy::input::gamepad::Gamepad>,
    panels: Res<super::panels::Panels>,
) {
    let (Some(assets), true) = (assets, control.pilot && seat.seated) else {
        *pilot_keys = PilotKeys::default();
        return;
    };
    let (mut window, mut cursor) = window.into_inner();
    let real = crate::mods::real_window(&cfg);
    let free = control.main_menu || (real && !window.focused);
    let pad = pads.iter().next();
    // (What was down when he sat down counts only once let go: the E that boarded doesn't thrust.)
    let (seen_keys, seen_buttons) = (unheld(&keys, &mut seat.held_keys), unheld(&buttons, &mut seat.held_buttons));
    *pilot_keys = if free { PilotKeys::default() } else { super::pilot::read_keys(&assets.keys, &assets.pad, pad, seen_keys.as_ref().unwrap_or(&keys), seen_buttons.as_ref().unwrap_or(&buttons), &motion, real && seat.regain == 0 && !seat.head_look, super::panels::UiSets { screen: seat.screen, ..panels.sets() }) };
    if !free {
        if keys.just_pressed(KeyCode::KeyB) {
            seat.leave = true;
        }
        // Head look (`HeadLookToggle`, the middle button in Elite's file).
        let toggle = assets.keys.get("HeadLookToggle").is_some_and(|a| a.keys().any(|k| k.device == "Mouse" && k.key == "Mouse_3"));
        if toggle && !seat.screen && buttons.just_pressed(MouseButton::Middle) {
            seat.head_look = !seat.head_look;
            if !seat.head_look {
                seat.look = Vec2::ZERO;
            }
        }
        if seat.head_look && !seat.screen && real && seat.regain == 0 {
            let d = motion.delta;
            seat.look.x = (seat.look.x - d.x * LOOK_RATE).clamp(-LOOK_YAW_MOST, LOOK_YAW_MOST);
            seat.look.y = (seat.look.y - d.y * LOOK_RATE).clamp(-LOOK_PITCH_MOST, LOOK_PITCH_MOST);
        }
    }
    // Every key's press but Esc's (the menu) and F11's (a screenshot): Elite's own are read above, the port's mustn't act.
    let taken: Vec<KeyCode> = keys.get_just_pressed().copied().filter(|k| !matches!(k, KeyCode::Escape | KeyCode::F11)).collect();
    for k in taken {
        keys.clear_just_pressed(k);
    }
    for b in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        buttons.clear_just_pressed(b);
        buttons.clear_just_released(b);
    }
    if real {
        let want = if free || seat.regain == 3 { CursorGrabMode::None } else { CursorGrabMode::Locked };
        if !free && seat.regain == 3 {
            let middle = Vec2::new(window.width(), window.height()) * 0.5;
            window.set_cursor_position(Some(middle));
        }
        if cursor.grab_mode != want {
            cursor.grab_mode = want;
        }
        cursor.visible = free;
    }
    seat.regain = seat.regain.saturating_sub(1);
}

/// The input as the ship sees it while some of what was held at boarding is still down (`held`, which loses each key or
/// button once it is let go): those reset as if up. None when nothing of it is still held.
fn unheld<T: Copy + Eq + std::hash::Hash + Send + Sync + 'static>(input: &ButtonInput<T>, held: &mut Vec<T>) -> Option<ButtonInput<T>> {
    held.retain(|k| input.pressed(*k));
    (!held.is_empty()).then(|| {
        let mut seen = input.clone();
        for k in held.iter() {
            seen.reset(*k);
        }
        seen
    })
}

/// The head's turn from straight ahead (`look`: radians, yaw to the left about the ship's up, then pitch up about the turned
/// x; no roll), in the ship's axes. The one function for it: `seat_camera` writes the camera with it and the HUD
/// (`hud_anchor::SeatView`) projects with it.
pub fn head_turn(look: Vec2) -> Quat {
    Quat::from_rotation_y(look.x) * Quat::from_rotation_x(look.y)
}

/// Elite's head look camera this frame (`HeadlookCamera`, its update `FUN_14107f170`; `re/elite/notes/sidewinder_hud.md`):
/// the head's turn (its `HeadlookCamYPR`: yaw and pitch, roll 0) and its field of view's channel (`HeadlookCamFov`,
/// radians, added to the configured view: `view_vertical_fov`). Elite aims the channel at the focused target's own value
/// (degrees in the target's data, x 0.0174533, clamped to +-pi/2) and at 0 with no target, so free head look and the head
/// straight ahead leave the configured view; its `HeadlookRelDisp` (the target's value along the turned view) is not built:
/// both panel pictures fit a head that does not move (-4 +- 8 mm, 1 +- 5 mm).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Head {
    pub turn: Vec2,
    pub fov_add: f32,
}

impl Head {
    /// A turn with no target: the configured view.
    pub fn turned(turn: Vec2) -> Head {
        Head { turn, fov_add: 0.0 }
    }
}

/// The seat's camera in the frame the ship's pose is given in (`pos`, `rot`: the ship space's origin and turn): at the eye
/// (`eye`, ship space), turned by the ship and then the head (`head_turn(head)`). What `seat_camera` writes.
pub fn eye_pose(pos: DVec3, rot: DQuat, eye: Vec3, head: Vec2) -> (DVec3, DQuat) {
    (pos + rot * eye.as_dvec3(), (rot * head_turn(head).as_dquat()).normalize())
}

/// The head's turn while a side panel is open by its key (radians, yaw left, pitch up), measured (grade C) on session ui1's
/// settled pictures with each panel open (`re/elite/measure/raw/ui1/shots`, 3440 x 1440 at the user's 56.249 degrees), the
/// way the role panel's is (`PANEL_TURN_ROLE`; `.optimize/scratch/run/ui5b/tools/role_turn.py`): Elite's HUD parts seen
/// both there and in the straight-ahead `a00_start.png` (at its own focal, 1365.2 px), the head turning about the eye.
/// - The right (`p2_right_3`, `_4`, `_5`, within 0.02 degrees of each other): the shield's 100 % and the fuel, ENG, SYS,
///   RST and WEP labels: 60.33 +- 0.20 degrees right, 13.60 +- 0.03 down, roll held 0 (free: 0.1, none), focal 2085.9 +-
///   17.0 px, residuals 0 to 4.3 px.
/// - The left (`p1_left_open_0`; `p2_left_2` gives 60.53 / 13.06): the info block's shipping-lane icon and the first
///   letters of its lines: 60.50 +- 0.16 degrees left (from the parts' places across), 13.28 down (+- 0.5: the block's lines
///   stand differently spaced in the two pictures, vertical residuals up to 22 px), the focal held at the right's (free, the
///   left's parts lie too close together to give it: 2463 +- 238 px).
/// So Elite aims the head at the panels' bones (the notes' 61 and 14 degrees, `re/elite/notes/ui_rendering.md` 4.3), the
/// two sides mirrors within 0.2 / 0.3 degrees, and whatever the window: one turn a panel (the mod's earlier rule that turned
/// the head further on narrower windows is gone). These replace the turns measured on the user's single screenshots
/// (`2.webp` left 38.13 / 7.95, `3.webp` right 36.04 / 2.84, S1b in `re/elite/notes/sidewinder_hud.md`): one picture each at
/// its own field of view, likely caught while the head was still turning (session ui1 timed the turn at about 1.1 s to
/// 90 %); they left both panels cut by the screen's edge, where ui1's settled pictures centre them.
pub const PANEL_TURN_LEFT: Vec2 = Vec2::new(1.05593, -0.23172);
pub const PANEL_TURN_RIGHT: Vec2 = Vec2::new(-1.05291, -0.23745);

/// The head's turn with the role panel focused (Elite's UI only, `SE_SW_UI=1`), measured (grade C) on session ui1's role
/// pictures (`re/elite/measure/raw/ui1/shots/p2_role_2..10.png` against the straight-ahead `a00_start.png`;
/// `.optimize/scratch/run/ui5b/tools/role_turn.py`): the canopy lamps are above that view, so the camera is fitted from eight
/// of Elite's HUD parts seen in both (the scanner's middle, the shield's and hull's 100 %, the fuel, pips and RST labels),
/// the head turning about the eye as `head_turn` does (Elite's data give the role target no displacement), a00 at its own
/// focal (its lamps against the mod's picture: 1.0135 x 1347.05 = 1365.2 px): yaw 0.02 +- 0.04 degrees, pitch 36.87 +- 0.04
/// down (+- 0.44 for a00's focal: 37.31 at the configured 1347.05), roll -0.06 +- 0.09 (none, as the head look camera's),
/// residuals 0 to 5 px (rms 1.6); the same on all nine pictures within 0.01 degrees.
pub const PANEL_TURN_ROLE: Vec2 = Vec2::new(0.00042, -0.64350);

/// The head's turn to a focused panel: the measured one, whatever the window (`PANEL_TURN_*`). The comms panel (Elite's UI
/// only) has no measured turn (session ui1 never focused it): the head stays straight ahead (a stand-in, grade E, until a
/// picture gives it).
pub fn panel_turn(side: Side) -> Vec2 {
    match side {
        Side::Left => PANEL_TURN_LEFT,
        Side::Right => PANEL_TURN_RIGHT,
        Side::Role => PANEL_TURN_ROLE,
        Side::Comms => Vec2::ZERO,
    }
}

/// The head look camera focused on an open panel: its turn (`panel_turn`) and the field of view's channel the stand-in
/// panels use (`panels::STAND_IN_FOV_DEG`, a copy of Elite's data). With Elite's UI the seat takes the focus machine's own
/// channel instead (`Panels::fov_add`, grade A: the data's -17 for the side and role panels, -8 for the comms panel; the
/// fits above gave -17.53 +- 0.29 for the right panel and -16.75 +- 0.1 for the role panel against the pictures' own view,
/// so they confirm the data within 2.5 sigma). The eye and the window no longer change it (kept for the callers).
pub fn panel_head(side: Side, _eye: Vec3, _aspect: f32) -> Head {
    Head { turn: panel_turn(side), fov_add: super::panels::STAND_IN_FOV_DEG.to_radians() }
}

/// The head look camera the panels' carrier asks for: at an open panel its turn (`panel_head`) with the carrier's channel
/// (`Panels::fov_add`); with no panel open, a channel Elite's focus machine gives (`Panels::fov_add_deg`: `Select UI Mode`'s
/// +10, a head look with no bone, B: the notes, `re/elite/notes/ui_rendering.md` 4.3, read its first pair as the one stored
/// for the camera unless the activity's +0x50 is -1 and +0x58 is not 0, two fields not decoded; the data's first pair is
/// taken) widens or narrows the view straight ahead; else none.
pub fn carrier_head(panels: &Panels, eye: Vec3, aspect: f32) -> Option<Head> {
    match (panels.open, panels.fov_add_deg) {
        (Some(side), _) => Some(Head { fov_add: panels.fov_add(), ..panel_head(side, eye, aspect) }),
        (None, Some(deg)) if deg != 0.0 => Some(Head { turn: Vec2::ZERO, fov_add: deg.to_radians() }),
        _ => None,
    }
}

/// A test's head turn as the seat takes it: any finite turn, as it is (no limits); None for one that is not finite (a NaN
/// or an infinity would leave the camera's turn undefined).
pub fn test_turn(turn: Vec2) -> Option<Vec2> {
    turn.is_finite().then_some(turn)
}

/// The head look camera this frame (`Seat::head`): a test's turn (`sw look`) over all, then head look's while it is on (both
/// without a target: the configured view), then the open side panel's (`panel`: `panel_head`'s), else straight ahead. The
/// change to a panel and back is at once: Elite eases the turn and the channel alike, each frame by min(dt, 0.05 s) x a rate
/// from the head look camera's data (`FUN_14107f170`), which is not at hand (a stand-in, grade E: no ease).
pub fn head_aim(test: Option<Vec2>, head_look: bool, look: Vec2, panel: Option<Head>) -> Head {
    match (test, head_look, panel) {
        (Some(t), ..) => Head::turned(t),
        (None, true, _) => Head::turned(look),
        (None, false, Some(p)) => p,
        (None, false, None) => Head::default(),
    }
}

/// The seat's camera at the pilot's eyes (bound to the body this frame), and every way out of the seat (`release`): B
/// (anywhere, `leave_rule`), the mod off by any way (in flight that loses the ship), the ship destroyed or removed.
#[allow(clippy::too_many_arguments)]
pub fn seat_camera(
    mut sb: ResMut<Sandbox>,
    mut seat: ResMut<Seat>,
    mut control: ResMut<ModControl>,
    mut ship: ResMut<Sidewinder>,
    assets: Option<Res<SidewinderAssets>>,
    mut cam: ResMut<FlyCamera>,
    bodies: Query<(&SystemBody, Option<&Terrain>)>,
    mut msg: ResMut<crate::messages::Messages>,
    mut pilot: ResMut<Pilot>,
    mut sim: ResMut<SimTime>,
    data: Res<crate::data::SeData>,
    mut panels: ResMut<Panels>,
    window: Query<&Window, With<PrimaryWindow>>,
) {
    // (Set again below where the camera is at the eye: every other way through, it isn't there. Likewise the tunnel's flag for
    // the universe's overlays, set only where the camera is in hyperspace's tunnel.)
    seat.view = false;
    control.tunnel = false;
    if !seat.seated {
        // Out, and not yet set down: the camera waits at the eye (the sandbox holds it there), so the seat's view, its draw set
        // and Elite's field of view, stays until the frame the sandbox has set him down.
        if seat.setting_down {
            seat.setting_down = sb.walking() && !sb.placed;
            seat.view = seat.setting_down;
        }
        return;
    }
    let Some(assets) = assets else { return };
    // Boarding is from foot only (`board`: E at the hull while walking): refused on the boarding frames alone, the parked
    // ship left as it stands (`boarding_refused`).
    if boarding_refused(seat.boarding, sb.mode) {
        free_seat(&mut seat, &mut sim, &mut control, &mut pilot, &mut panels);
        seat.setting_down = false;
        msg.show("Board the Sidewinder on foot".to_string());
        info!("sidewinder: boarding refused: not on foot");
        return;
    }
    if sb.mode == Mode::Pilot {
        seat.boarding = false;
    }
    // Where the seat's camera is this frame (the tunnel first, then the ship's body): the camera below and the release both
    // go by it.
    let at = seat_at(&pilot, &ship, &bodies);
    // Every way out: the ship removed (or gone), destroyed, the mod turned off by any way, B (`leave_rule`).
    let leave = |by_b: bool| {
        let g = ground_under(&at, &assets.shape);
        let reach = within_step_out(g.level, g.lowest_m, pilot.cruise.is_some(), pilot.jump.is_some());
        Why::from(leave_rule(pilot.landed, reach, by_b))
    };
    let why = if seat.remove || ship.parked.is_none() {
        Some(Why::Removed)
    } else if pilot.destroyed {
        Some(Why::Destroyed)
    } else if !control.pilot {
        Some(leave(false))
    } else if std::mem::take(&mut seat.leave) {
        if sb.mode == Mode::Pilot {
            Some(leave(true))
        } else {
            // (The boarding frames: a ship he has not yet sat in is never abandoned, nor got out of.)
            info!("sidewinder: B dropped: the seat does not have its mode yet (boarding)");
            None
        }
    } else {
        None
    };
    if let Some(why) = why {
        let mut out = Out { seat: &mut *seat, sim: &mut *sim, control: &mut *control, pilot: &mut *pilot, ship: &mut *ship, sb: &mut *sb, cam: &mut *cam, msg: &mut *msg, panels: &mut *panels };
        release(why, &mut out, &at, &data, &assets);
        // (The camera is at the eye this frame: the seat's view until he is set down.)
        seat.view = seat.setting_down && sb.walking() && !sb.placed;
        return;
    }
    let eye = Vec3::from_array(assets.eye);
    let aspect = window.single().map_or(ELITE_ASPECT, |w| w.width() / w.height().max(1.0));
    if let Some(bad) = seat.test_look.filter(|t| test_turn(*t).is_none()) {
        warn!("sidewinder: a test's head turn of {:?} degrees is not a number: refused, the head is the pilot's", bad * 180.0 / std::f32::consts::PI);
        seat.test_look = None;
    }
    seat.head = if seat.screen { Head::default() } else { head_aim(seat.test_look, seat.head_look, seat.look, carrier_head(&panels, eye, aspect)) };
    match at {
        // In hyperspace's tunnel the ship's pose is the jump's (in the universe: the old system's bodies go meanwhile), held by
        // `hyper::tunnel`; the camera is at the eye there, unbound, the head turned as in flight.
        SeatAt::Tunnel(_) => {
            write_seat_camera(&mut cam, &at, eye, seat.head.turn);
            seat.view = true;
            control.tunnel = true;
        }
        SeatAt::Body { .. } if control.pilot && sb.mode == Mode::Pilot => {
            write_seat_camera(&mut cam, &at, eye, seat.head.turn);
            seat.view = true;
        }
        // (Otherwise boarded this frame: the sandbox leaves first person at the end of its step; the seat takes the camera next
        // frame.)
        _ => {}
    }
}

/// Writes the camera at the seat's place (`seat_at`), at the eye (`eye`, ship space) with the head's turn (`head`): in
/// hyperspace's tunnel at the eye of the jump's pose in the universe, unbound and at rest; by a body at the eye, bound to it
/// as of this frame (the 500 m-a-frame rule), and the eye's pose in the body's frame is returned. With no place nothing is
/// written. The one writer `seat_camera` and `release` share, so the release never moves the camera's place (the eye's
/// place does not depend on the head's turn).
fn write_seat_camera(cam: &mut FlyCamera, at: &SeatAt, eye: Vec3, head: Vec2) -> Option<(DVec3, DQuat)> {
    match at {
        SeatAt::Tunnel(j) => {
            let (eye_at, rot) = eye_pose(DVec3::ZERO, j.rot, eye, head);
            cam.pos = j.arrive.offset_m(eye_at);
            cam.rot = rot;
            cam.velocity = DVec3::ZERO;
            cam.travel = None;
            cam.track = false;
            cam.hold_agl = None;
            cam.follow = None;
            None
        }
        SeatAt::Body { body: b, pos, rot, .. } => {
            let (eye_body, rot_body) = eye_pose(*pos, *rot, eye, head);
            cam.pos = b.pos.offset_m(b.orient * eye_body);
            cam.rot = (b.orient * rot_body).normalize();
            crate::sandbox::bind_camera(cam, b);
            Some((eye_body, rot_body))
        }
        SeatAt::Nowhere => None,
    }
}

/// What the seat's release changes (borrowed from `seat_camera`'s parameters).
struct Out<'a> {
    seat: &'a mut Seat,
    sim: &'a mut SimTime,
    control: &'a mut ModControl,
    pilot: &'a mut Pilot,
    ship: &'a mut Sidewinder,
    sb: &'a mut Sandbox,
    cam: &'a mut FlyCamera,
    msg: &'a mut crate::messages::Messages,
    panels: &'a mut Panels,
}

/// Releases the seat, whichever way he leaves it (`Why`): `free_seat`'s state changes; the camera written once more this
/// frame where the seat's camera is (`write_seat_camera` at `at`: the release never moves its place), turned as the ship
/// without the head's turn, level by the ship's heading out of a ship that was not landed (`out_turn`: also the pose
/// `Sandbox::set_down` is given); he is put down beside the
/// ship when it stays (landed or within reach of the ground: `keep_in_world` sets it down on its feet), in the free flight
/// where an abandoned one was, on the ground under a lost one, else on Earth (`exit_rule`); a lost or removed ship goes,
/// solids and all (a ship left behind in flight would hang in the air as a solid thing).
fn release(why: Why, out: &mut Out, at: &SeatAt, data: &crate::data::SeData, assets: &SidewinderAssets) {
    // (The camera's place is the seat's, its turn the ship's own without the head's, as before A12, or its level heading out of
    // a ship that was not landed (`out_turn`): out of the seat he faces along the ship, not where his head looked. The fade
    // from black covers the change of view.)
    let head = Vec2::ZERO;
    let in_jump = out.pilot.jump.is_some();
    let in_cruise = out.pilot.cruise.is_some();
    let landed = out.pilot.landed;
    let g = ground_under(at, &assets.shape);
    let ground = ground_by_ship(g.level, in_cruise, in_jump);
    let (lost, place) = exit_rule(why, landed, ground, out.ship.parked.is_some(), out.sb.mode == Mode::Pilot);
    free_seat(out.seat, out.sim, out.control, out.pilot, out.panels);
    let eye = Vec3::from_array(assets.eye);
    match (place, at) {
        (Place::Flight, _) => {
            // (With no place, the camera stays where it is, unbound and at rest.)
            if write_seat_camera(out.cam, at, eye, head).is_none() {
                out.cam.velocity = DVec3::ZERO;
                out.cam.travel = None;
                out.cam.follow = None;
            }
            out.sb.free_flight(out.cam, flight_by_ground(g.level, g.origin_m, in_cruise, in_jump));
        }
        (Place::Beside | Place::Under, SeatAt::Body { body: b, pos, rot, .. }) => {
            // (The camera at the seat's place; its turn, and the pose he is set down with, `out_turn`'s.)
            let turn = out_turn(place, landed, *pos, *rot);
            let eye_body = write_seat_camera(out.cam, at, eye, head).map_or(*pos, |(e, _)| e);
            out.cam.rot = (b.orient * turn).normalize();
            // Beside: out from the hull's left side, as the ship will stand (not landed: in its level heading frame, the heading
            // `keep_in_world` sets it down with); under: the ship's own place (the sandbox drops the point to the ground).
            let beside = Vec3::new(assets.shape.lo.x - EXIT_OUT_M, 1.0, (assets.shape.lo.z + assets.shape.hi.z) * 0.5).as_dvec3();
            let point = if place == Place::Beside { *pos + turn * beside } else { *pos };
            out.sb.set_down(b.obj, point, None, eye_body, turn);
        }
        (Place::Beside | Place::Under | Place::Earth, _) => {
            // (`put_on_earth` writes the camera at the spot it waits at, bound to Earth.)
            crate::sandbox::put_on_earth(data, out.sim.jd, out.cam, out.sb, "out of the Sidewinder far from ground beside it: back");
        }
        (Place::Stay, _) => {}
    }
    if lost {
        if let Some(mut gone) = out.ship.parked.take() {
            super::take_out(out.sb, &mut gone);
        }
    } else {
        // The ship stays, set down on its feet at rest (`keep_in_world`): nothing of the flight may go on thinking it flies.
        out.pilot.landed = true;
    }
    out.control.ship_parked = out.ship.parked.is_some();
    let where_ = if in_jump { " in hyperspace" } else { "" };
    let text = match why {
        Why::Destroyed => format!("The Sidewinder was destroyed{where_}"),
        Why::Removed => "The Sidewinder was removed".to_string(),
        Why::Abandon => format!("Disembarked{where_}: the Sidewinder is gone"),
        Why::Lost => format!("The Sidewinder was lost{where_}"),
        Why::Out => "Disembarked".to_string(),
    };
    out.msg.show(text.clone());
    info!("sidewinder: the seat is released ({why:?}): {text}; {}; over the ground: the ship's origin {:.2} m, its lowest point {:.2} m", match place {
        Place::Beside => "out, beside the ship",
        Place::Under => "out, on the ground under the ship",
        Place::Earth => "back on Earth",
        Place::Flight => "in the free flight where the ship was",
        Place::Stay => "already out of the seat's mode",
    }, g.origin_m, g.lowest_m);
}

/// The `<FOV>` of the user's Elite settings (`Options/Graphics/Settings.xml`), degrees (grade B: the user's file; the mod
/// never reads Elite's files at run time). Elite reads it clamped to 54.32..120 (`FUN_142858180`: the `FOV` element into
/// the graphics options' float at +0x5c, its limits the floats at 0x1452f5f1c and 0x144de16a0) and the cockpit's camera node
/// (`ConfiguredFOVCamera`, named "FOV": `FUN_141083010`) turns it into radians (`FUN_141030030`: x 0.0174533).
pub const SEAT_FOV_DEG: f32 = 56.249;

/// Elite's reference aspect (`FUN_142840260`'s float at 0x1452f5f14): at and above it the setting is the vertical field of
/// view; narrower, the horizontal one this aspect would have is kept.
const ELITE_ASPECT: f32 = 16.0 / 9.0;

/// The seat's vertical field of view (radians) on a picture `aspect` wide (width / height), by Elite's rule
/// (`FUN_142840260`, the graphics device's field of view for an aspect, called with the device's own aspect from its +0x70
/// or +0x74 by `FUN_1428436c0` / `FUN_1428441c0`): 2 atan(tan(fov / 2) x max(16/9 / aspect, 1)), fov the setting in
/// radians (Elite skips it in VR or with the device's flag bit 6). So 56.249 degrees at 16:9 and on every wider screen
/// (3440 x 1440: a focal of 1347.05 px), wider vertically on a narrower one (4:3: 70.95 degrees). The configured view: the
/// head look camera with no target (`view_vertical_fov` with no channel).
pub fn seat_vertical_fov(aspect: f32) -> f32 {
    view_vertical_fov(aspect, 0.0)
}

/// The seat's vertical field of view (radians) on a picture `aspect` wide with the head look camera's field of view's
/// channel `fov_add` (radians, `Head::fov_add`) added to the configured value before the device's rule for the aspect
/// (`seat_vertical_fov`'s): 2 atan(tan((fov + fov_add) / 2) x max(16/9 / aspect, 1)). (That the channel adds is read from
/// its form: 0 without a target, clamped to +-pi/2; how Elite's camera graph combines it is data, not traced.)
pub fn view_vertical_fov(aspect: f32, fov_add: f32) -> f32 {
    let k = (ELITE_ASPECT / aspect.max(1e-3)).max(1.0);
    2.0 * ((0.5 * (SEAT_FOV_DEG.to_radians() + fov_add)).tan() * k).atan()
}

/// While the camera is at the eye (`Seat::view`), the main camera's vertical field of view is Elite's for the window's
/// aspect and the head look camera's channel (`view_vertical_fov`): written into its projection after everything else that
/// sets it this frame (the flight's `FlyCamera::fov_deg` by `sync_camera_transform`, then the sandbox's first-person view),
/// before its matrices are made and before the capture views copy it (the mod's `plugin` orders it). The player's own setting
/// (`fov_deg`, saved with the settings) is never touched, so nothing has to be given back and nothing can leak: from the
/// frame the camera leaves the eye (`seat_camera` clears `view` once the sandbox has set him down), whichever way he left,
/// the view's own field of view is drawn again. Every change is logged.
pub fn seat_fov(seat: Res<Seat>, window: Query<&Window, With<PrimaryWindow>>, mut proj: Query<&mut Projection, With<crate::camera::MainCamera>>, mut was: Local<(bool, f32)>) {
    let aspect = window.single().map_or(ELITE_ASPECT, |w| w.width() / w.height().max(1.0));
    let fov = view_vertical_fov(aspect, seat.head.fov_add);
    for mut p in &mut proj {
        let Projection::Perspective(p) = p.as_mut() else { continue };
        if seat.view != was.0 {
            let own = p.fov.to_degrees();
            if seat.view {
                info!("sidewinder: the seat's view: vertical field of view {:.3} degrees at the aspect {aspect:.3} (the view's own was {own:.3})", fov.to_degrees());
            } else {
                info!("sidewinder: out of the seat's view: the view's own field of view again, {own:.3} degrees");
            }
        } else if seat.view && seat.head.fov_add != was.1 {
            info!("sidewinder: the seat's view: vertical field of view {:.3} degrees (the configured {:.3}, the head look's channel {:+.3})", fov.to_degrees(), seat_vertical_fov(aspect).to_degrees(), seat.head.fov_add.to_degrees());
        }
        if seat.view {
            p.fov = fov;
        }
    }
    *was = (seat.view, seat.head.fov_add);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The E that boards (and whatever else is down then) does nothing in the ship until it has been let go once; pressed
    /// again it is the ship's key (E is the thrust to the right).
    #[test]
    fn the_boarding_press_counts_only_once_let_go() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyE);
        keys.press(KeyCode::KeyW);
        let mut sim = SimTime::default();
        let mut seat = Seat::boarded(&mut sim, &keys, &ButtonInput::default());
        keys.clear();
        // Still held a frame later: the ship sees neither.
        let seen = unheld(&keys, &mut seat.held_keys).expect("held at boarding");
        assert!(!seen.pressed(KeyCode::KeyE) && !seen.pressed(KeyCode::KeyW));
        // E let go, W still down: W stays out.
        keys.release(KeyCode::KeyE);
        let seen = unheld(&keys, &mut seat.held_keys).expect("W still held");
        assert!(!seen.pressed(KeyCode::KeyW) && seat.held_keys == vec![KeyCode::KeyW]);
        // E pressed again counts; once W is up nothing is masked.
        keys.press(KeyCode::KeyE);
        keys.release(KeyCode::KeyW);
        assert!(unheld(&keys, &mut seat.held_keys).is_none() && keys.pressed(KeyCode::KeyE));
    }

    /// A12 (replacing the spec's C13): landed or within reach of the ground, out (the ship stays), whichever way he leaves;
    /// anywhere else B or Disembark abandons the ship and the mod going off any other way loses it. Nothing refuses.
    #[test]
    fn leaving_works_anywhere() {
        // (landed, within reach to step out, by B): every row.
        for by_b in [true, false] {
            assert_eq!(leave_rule(true, true, by_b), Leave::Out);
            assert_eq!(leave_rule(true, false, by_b), Leave::Out);
            assert_eq!(leave_rule(false, true, by_b), Leave::Out);
        }
        assert_eq!(leave_rule(false, false, true), Leave::Abandon);
        assert_eq!(leave_rule(false, false, false), Leave::Lost);
        assert_eq!(Why::from(Leave::Out), Why::Out);
        assert_eq!(Why::from(Leave::Abandon), Why::Abandon);
        assert_eq!(Why::from(Leave::Lost), Why::Lost);
    }

    /// Within reach to step out is the boarding's reach over ground he can stand on (A12): never in supercruise or the tunnel.
    #[test]
    fn within_reach_to_step_out_is_the_boarding_reach() {
        assert!(within_step_out(Some(14), 5.9, false, false));
        assert!(!within_step_out(Some(14), 6.1, false, false));
        assert!(!within_step_out(None, 1.0, false, false));
        assert!(!within_step_out(Some(14), 1.0, true, false));
        assert!(!within_step_out(Some(14), 1.0, false, true));
    }
    /// The model box's lowest corner (what `ground_under` measures the reach by) in ship space (X right, Y up, -Z the nose):
    /// level, the box's bottom; turned 90 degrees about the nose, half its width under the origin; upside down, its top.
    #[test]
    fn the_lowest_corner_of_a_turned_box() {
        let (lo, hi) = (Vec3::new(-6.0, 0.5, -7.0), Vec3::new(6.0, 3.0, 8.0));
        let corners = box_corners(lo, hi);
        assert_eq!(corners.len(), 8);
        for c in [lo, hi, Vec3::new(lo.x, hi.y, lo.z), Vec3::new(hi.x, lo.y, hi.z)] {
            assert!(corners.contains(&c), "{c:?}");
        }
        let lowest = |rot: DQuat| corners.iter().map(|c| (rot * c.as_dvec3()).y).fold(f64::INFINITY, f64::min);
        assert!((lowest(DQuat::IDENTITY) - 0.5).abs() < 1e-9);
        assert!((lowest(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)) + 6.0).abs() < 1e-9);
        assert!((lowest(DQuat::from_rotation_z(std::f64::consts::PI)) + 3.0).abs() < 1e-9);
    }

    /// Out of a ship that was not landed he steps out by its level heading frame: a level ship's is its own turn (today's
    /// point), rolled 90 or 180 degrees about its nose it is the level one, and with the nose straight up or down it is
    /// still a finite frame (the belly's or top's heading).
    #[test]
    fn the_level_frame_keeps_the_heading() {
        let pos = DVec3::new(0.0, 6_371_000.0, 0.0);
        let beside = DVec3::new(-8.5, 1.0, 0.5);
        let point = |q: DQuat| pos + q * beside;
        let level = DQuat::from_rotation_y(0.7);
        assert!(point(level_frame(pos, level)).distance(point(level)) < 1e-6);
        for roll in [std::f64::consts::FRAC_PI_2, std::f64::consts::PI] {
            let rolled = level * DQuat::from_rotation_z(roll);
            assert!(point(level_frame(pos, rolled)).distance(point(level)) < 1e-6, "{roll}");
        }
        for pitch in [std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2] {
            let q = level_frame(pos, level * DQuat::from_rotation_x(pitch));
            assert!(q.is_finite() && (q.length() - 1.0).abs() < 1e-9, "{pitch}");
            assert!((q * DVec3::Y).distance(DVec3::Y) < 1e-9, "up stays the radial: {pitch}");
        }
    }

    /// The turn handed over at the release: out of a ship that was not landed it is level (its forward has no part along the
    /// radial) along the ship's heading, for a ship pitched 66 degrees nose up or down and one on its tail or its nose (whose
    /// heading is the belly's or the top's); landed, and under a lost ship, it is the ship's own turn exactly.
    #[test]
    fn out_of_a_tipped_ship_he_faces_its_heading_level() {
        let pos = DVec3::new(0.0, 6_371_000.0, 0.0);
        let heading = DQuat::from_rotation_y(0.7);
        for deg in [66.0f64, -66.0, 90.0, -90.0] {
            let tipped = heading * DQuat::from_rotation_x(deg.to_radians());
            let fwd = out_turn(Place::Beside, false, pos, tipped) * DVec3::NEG_Z;
            assert!(fwd.y.abs() < 1e-9, "{deg}: the forward's pitch {} degrees", fwd.y.asin().to_degrees());
            assert!(fwd.distance(heading * DVec3::NEG_Z) < 1e-6, "{deg}: the heading {fwd:?}");
            assert_eq!(out_turn(Place::Beside, true, pos, tipped), tipped, "{deg}: landed");
            assert_eq!(out_turn(Place::Under, false, pos, tipped), tipped, "{deg}: under");
        }
    }

    /// Abandoned (A12): in the seat's mode the ship is lost and he is in the free flight, whatever the ground (and whether a
    /// body was found); outside it he stays where he is.
    #[test]
    fn an_abandon_gives_the_free_flight() {
        for ground in [true, false] {
            for ship in [true, false] {
                assert_eq!(exit_rule(Why::Abandon, false, ground, ship, true), (true, Place::Flight), "{ground} {ship}");
            }
        }
        assert_eq!(exit_rule(Why::Abandon, false, true, true, false), (true, Place::Stay));
    }

    /// The free flight's speed starts at V's pace only by ground within the sandbox's reach of the ship (A12).
    #[test]
    fn by_ground_for_the_free_flight_is_within_reach() {
        assert!(flight_by_ground(Some(14), 1499.0, false, false));
        assert!(!flight_by_ground(Some(14), 1501.0, false, false));
        assert!(!flight_by_ground(None, 100.0, false, false));
        assert!(!flight_by_ground(Some(14), 100.0, true, false));
        assert!(!flight_by_ground(Some(14), 100.0, false, true));
    }

    /// The release's rule (F6): out (landed, or within reach of the ground though not landed) keeps the ship and sets him down
    /// beside it; lost in flight he goes to the ground under it, or to Earth with no ground to stand on; destroyed or removed,
    /// the ship goes; outside the seat's mode he stays where he is.
    #[test]
    fn the_release_rule() {
        // (why, landed, ground, ship, in the seat's mode)
        assert_eq!(exit_rule(Why::Out, true, true, true, true), (false, Place::Beside));
        assert_eq!(exit_rule(Why::Out, false, true, true, true), (false, Place::Beside));
        assert_eq!(exit_rule(Why::Lost, false, true, true, true), (true, Place::Under));
        assert_eq!(exit_rule(Why::Lost, false, false, true, true), (true, Place::Earth));
        assert_eq!(exit_rule(Why::Destroyed, true, true, true, true), (true, Place::Beside));
        assert_eq!(exit_rule(Why::Destroyed, false, true, true, true), (true, Place::Under));
        assert_eq!(exit_rule(Why::Destroyed, false, false, true, true), (true, Place::Earth));
        assert_eq!(exit_rule(Why::Removed, true, true, true, true), (true, Place::Beside));
        assert_eq!(exit_rule(Why::Removed, false, true, true, true), (true, Place::Under));
        assert_eq!(exit_rule(Why::Removed, true, true, false, true), (true, Place::Earth));
        assert_eq!(exit_rule(Why::Out, true, true, true, false), (false, Place::Stay));
        assert_eq!(exit_rule(Why::Lost, false, true, true, false), (true, Place::Stay));
    }

    /// Ground to stand on by the ship is any loaded ground of a solid body, the sea's surface included (no sea test: landed on
    /// the sea, B sets him down beside the ship, never back at the game's start); none in supercruise or hyperspace, or with
    /// nothing loaded (or no solid body) under it.
    #[test]
    fn the_sea_is_ground_to_get_out_on() {
        assert!(ground_by_ship(Some(14), false, false));
        assert_eq!(exit_rule(Why::Out, true, ground_by_ship(Some(14), false, false), true, true), (false, Place::Beside));
        for (level, cruise, jump) in [(None, false, false), (Some(14), true, false), (Some(14), false, true)] {
            assert!(!ground_by_ship(level, cruise, jump), "{level:?} {cruise} {jump}");
        }
    }

    /// A boarding is refused only before the seat's mode first comes and only away from foot; seated in the seat's mode, any
    /// other mode (another mod turned on in flight) is not a refusal but a way out, which loses a flying ship.
    #[test]
    fn only_a_boarding_is_refused() {
        assert!(boarding_refused(true, Mode::Fly) && boarding_refused(true, Mode::Hero));
        assert!(!boarding_refused(true, Mode::Walk) && !boarding_refused(true, Mode::Pilot));
        for mode in [Mode::Walk, Mode::Fly, Mode::Hero, Mode::Pilot] {
            assert!(!boarding_refused(false, mode), "{mode:?}");
        }
        let mut sim = SimTime { jd: 2_460_000.0, rate: 1.0, paused: false };
        assert!(Seat::boarded(&mut sim, &ButtonInput::default(), &ButtonInput::default()).boarding);
        // Another mod on in flight: out of reach of the ground the release's rule loses the ship; within reach it stays, and
        // he stays where the other mod put him.
        assert_eq!(exit_rule(Why::from(leave_rule(false, false, false)), false, true, true, false).0, true);
        assert_eq!(exit_rule(Why::from(leave_rule(false, true, false)), false, true, true, false), (false, Place::Stay));
    }

    /// The release's state changes: out of the seat, the old clock given back, head look, a test's head turn, an open side
    /// panel and a pending B or removal cleared, the fade started, the flight's transient states ended; the ship's systems
    /// kept.
    #[test]
    fn the_release_frees_the_seat() {
        let mut sim = SimTime { jd: 2_460_000.0, rate: 1000.0, paused: true };
        let mut seat = Seat::boarded(&mut sim, &ButtonInput::default(), &ButtonInput::default());
        assert_eq!((sim.rate, sim.paused), (1.0, false));
        seat.head_look = true;
        seat.look = Vec2::new(0.5, -0.2);
        seat.test_look = Some(Vec2::new(0.4, 0.1));
        seat.head = panel_head(Side::Right, EYE, 3440.0 / 1440.0);
        seat.request_leave();
        seat.request_remove();
        let mut panels = Panels::default();
        panels.update(&super::super::panels::PanelKeys { right: true, ..default() }, None);
        assert_eq!(panels.open, Some(Side::Right));
        let mut control = ModControl::default();
        control.pilot = true;
        control.active = Some(CODE.to_string());
        let mut pilot = Pilot::default();
        pilot.glide = Some(elite_sim::glide::Glide::default());
        pilot.fuel.main = 1.25;
        free_seat(&mut seat, &mut sim, &mut control, &mut pilot, &mut panels);
        assert!(!seat.seated && !seat.head_look && seat.look == Vec2::ZERO && !seat.leave && !seat.remove);
        // Boarding again starts straight ahead at the configured view: no test turn, no panel.
        assert!(seat.test_look.is_none() && seat.head == Head::default() && panels.open.is_none() && !panels.by_look);
        assert!(seat.setting_down && seat.fade == FADE_S && seat.clock.is_none());
        assert_eq!((sim.rate, sim.paused), (1000.0, true));
        assert!(pilot.glide.is_none() && pilot.cruise.is_none() && pilot.jump.is_none());
        assert_eq!(pilot.fuel.main, 1.25);
        // A second release (nothing kept of the clock) leaves the clock as it is.
        sim.rate = 5.0;
        free_seat(&mut seat, &mut sim, &mut control, &mut pilot, &mut panels);
        assert_eq!(sim.rate, 5.0);
    }

    /// In the seat's view the projection takes Elite's field of view, narrowed by the head look's channel while a panel is
    /// focused; out of it the seat leaves the view's own (written each frame by the camera's systems before it) as it is.
    #[test]
    fn the_seat_sets_elites_field_of_view_and_leaves_the_players() {
        use bevy::ecs::system::RunSystemOnce;
        let own = 75f32.to_radians();
        let mut world = World::new();
        let cam = world.spawn((crate::camera::MainCamera, Projection::Perspective(PerspectiveProjection { fov: own, ..default() }))).id();
        let fov = |w: &World| match w.get::<Projection>(cam) {
            Some(Projection::Perspective(p)) => p.fov,
            _ => panic!("no perspective"),
        };
        world.insert_resource(Seat { seated: true, view: true, ..default() });
        world.run_system_once(seat_fov).unwrap();
        // (No window here: the rule's reference aspect, 16:9.)
        assert!((fov(&world).to_degrees() - SEAT_FOV_DEG).abs() < 1e-4, "{}", fov(&world).to_degrees());
        world.resource_mut::<Seat>().head = panel_head(Side::Left, EYE, ELITE_ASPECT);
        world.run_system_once(seat_fov).unwrap();
        assert!((fov(&world).to_degrees() - (SEAT_FOV_DEG + super::super::panels::STAND_IN_FOV_DEG)).abs() < 1e-4, "{}", fov(&world).to_degrees());
        // Out of the seat (any way: `view` is false), after the camera's systems wrote the view's own again.
        if let Some(mut p) = world.get_mut::<Projection>(cam) {
            *p = Projection::Perspective(PerspectiveProjection { fov: own, ..default() });
        }
        world.resource_mut::<Seat>().view = false;
        world.run_system_once(seat_fov).unwrap();
        assert_eq!(fov(&world), own);
    }

    /// Elite's rule (`FUN_142840260`): the setting is the vertical field of view at 16:9 (1600 x 900, 1920 x 1080) and on any
    /// wider screen (3440 x 1440: a focal of 1347.05 px on its 1,440 lines; session 2's whole turns measured 1,347 and
    /// session 1's frames' canopy lamps 1,355 to 1,362); narrower, 16:9's horizontal angle is kept (4:3: 70.95 degrees).
    #[test]
    fn the_field_of_view_follows_elites_rule() {
        for (w, h) in [(1600.0f32, 900.0f32), (1920.0, 1080.0), (3440.0, 1440.0), (2560.0, 1080.0)] {
            assert!((seat_vertical_fov(w / h).to_degrees() - 56.249).abs() < 1e-4, "{w}x{h}");
        }
        let focal = 720.0 / (0.5 * seat_vertical_fov(3440.0 / 1440.0)).tan();
        assert!((focal - 1347.05).abs() < 0.05, "{focal}");
        let narrow = seat_vertical_fov(4.0 / 3.0);
        assert!((narrow.to_degrees() - 70.95).abs() < 0.01, "{}", narrow.to_degrees());
        // The horizontal angle at 4:3 is 16:9's.
        let across = |v: f32, a: f32| 2.0 * ((0.5 * v).tan() * a).atan();
        assert!((across(narrow, 4.0 / 3.0) - across(seat_vertical_fov(16.0 / 9.0), 16.0 / 9.0)).abs() < 1e-5);
    }

    /// A focused panel's view (`view_vertical_fov` with its channel): the configured 56.249 degrees plus Elite's channel at
    /// 16:9 and wider (the side panels' -17: 39.249), the same rule for the aspect below (4:3: 16:9's horizontal angle of
    /// that view); no channel, the configured view. The seat takes the focus machine's channel when Elite's UI gives one
    /// (the comms panel's -8), else the stand-in's copy of the data.
    #[test]
    fn a_focused_panel_narrows_the_view_by_its_channel() {
        let deg = |side, aspect| view_vertical_fov(aspect, panel_head(side, EYE, aspect).fov_add).to_degrees();
        for (w, h) in [(1600.0f32, 900.0f32), (1920.0, 1080.0), (3440.0, 1440.0)] {
            assert!((deg(Side::Left, w / h) - 39.249).abs() < 1e-3, "{w}x{h}: {}", deg(Side::Left, w / h));
            assert!((deg(Side::Right, w / h) - 39.249).abs() < 1e-3, "{w}x{h}: {}", deg(Side::Right, w / h));
        }
        let across = |v: f32, a: f32| 2.0 * ((0.5 * v).tan() * a).atan();
        let narrow = deg(Side::Left, 4.0 / 3.0).to_radians();
        assert!((across(narrow, 4.0 / 3.0) - across(39.249f32.to_radians(), 16.0 / 9.0)).abs() < 1e-4);
        assert_eq!(view_vertical_fov(4.0 / 3.0, 0.0), seat_vertical_fov(4.0 / 3.0));
        let mut panels = Panels { open: Some(Side::Comms), ..default() };
        assert!((panels.fov_add().to_degrees() - super::super::panels::STAND_IN_FOV_DEG).abs() < 1e-5, "the stand-in's copy");
        panels.fov_add_deg = Some(-8.0);
        assert!((panels.fov_add().to_degrees() + 8.0).abs() < 1e-5, "the machine's own");
    }

    /// The carrier's head: at a panel its turn with the machine's channel (the stand-in's copy only without one); with no
    /// panel, a channel the machine gives alone (`Select UI Mode`'s +10) straight ahead; 0 or the stand-in path: none.
    #[test]
    fn the_carrier_asks_for_its_head() {
        let at = |open, deg| carrier_head(&Panels { open, fov_add_deg: deg, ..default() }, EYE, 16.0 / 9.0);
        assert_eq!(at(None, None), None);
        assert_eq!(at(None, Some(0.0)), None, "the comms bar, the flight");
        assert_eq!(at(None, Some(10.0)), Some(Head { turn: Vec2::ZERO, fov_add: 10f32.to_radians() }), "Select UI Mode");
        assert_eq!(at(Some(Side::Comms), Some(-8.0)), Some(Head { turn: Vec2::ZERO, fov_add: (-8f32).to_radians() }));
        assert_eq!(at(Some(Side::Left), Some(-17.0)), Some(Head { turn: PANEL_TURN_LEFT, fov_add: (-17f32).to_radians() }));
        assert_eq!(at(Some(Side::Left), None).map(|h| h.fov_add), Some(super::super::panels::STAND_IN_FOV_DEG.to_radians()), "the stand-in");
    }

    /// One turn a panel, whatever the window (Elite aims at the panel's bone): the measured ones; the comms panel straight.
    #[test]
    fn a_panels_turn_is_the_measured_one() {
        for (w, h) in [(1024.0f32, 768.0f32), (1600.0, 900.0), (3440.0, 1440.0), (5120.0, 1440.0)] {
            assert_eq!(panel_head(Side::Left, EYE, w / h).turn, PANEL_TURN_LEFT);
            assert_eq!(panel_head(Side::Right, EYE, w / h).turn, PANEL_TURN_RIGHT);
            assert_eq!(panel_head(Side::Role, EYE, w / h).turn, PANEL_TURN_ROLE);
            assert_eq!(panel_head(Side::Comms, EYE, w / h).turn, Vec2::ZERO);
        }
        assert!((PANEL_TURN_LEFT.x.to_degrees() - 60.50).abs() < 0.01 && (PANEL_TURN_RIGHT.x.to_degrees() + 60.33).abs() < 0.01);
    }

    /// A panel opened by its key turns the head to it with its narrower view and closing it gives the head back; the pilot's
    /// own head look and a test's turn come first, both at the configured view (no target); getting out leaves the head
    /// straight and the panel closed.
    #[test]
    fn a_panel_turns_the_head_and_closing_gives_it_back() {
        let own = Vec2::new(0.3, -0.1);
        let head = |side| panel_head(side, EYE, 16.0 / 9.0);
        assert_eq!(head_aim(None, false, Vec2::ZERO, None), Head::default());
        assert_eq!(head_aim(None, false, Vec2::ZERO, Some(head(Side::Left))), head(Side::Left));
        assert_eq!(head_aim(None, true, own, Some(head(Side::Left))), Head::turned(own));
        assert_eq!(head_aim(Some(Vec2::X), true, own, Some(head(Side::Left))), Head::turned(Vec2::X));
        // Through the panels' own keys: 1 opens the left, 1 again closes it.
        let mut panels = Panels::default();
        let one = super::super::panels::PanelKeys { left: true, ..default() };
        panels.update(&one, None);
        assert_eq!(head_aim(None, false, Vec2::ZERO, panels.open.map(head)), head(Side::Left));
        panels.update(&one, None);
        assert_eq!(head_aim(None, false, Vec2::ZERO, panels.open.map(head)), Head::default());
        // The turns point at their panels, left and down, right and down, and each narrows the view.
        let (l, r) = (head(Side::Left), head(Side::Right));
        assert!(l.turn.x > 0.6 && r.turn.x < -0.6 && l.turn.y < 0.0 && r.turn.y < 0.0 && l.fov_add < 0.0 && r.fov_add < 0.0);
        let mut sim = SimTime { jd: 2_460_000.0, rate: 1.0, paused: false };
        let mut seat = Seat::boarded(&mut sim, &ButtonInput::default(), &ButtonInput::default());
        seat.head = l;
        panels.update(&one, None);
        free_seat(&mut seat, &mut sim, &mut ModControl::default(), &mut Pilot::default(), &mut panels);
        assert_eq!((seat.head, panels.open), (Head::default(), None));
    }

    /// The eye: the cockpit's `!Camera` bone in the ship's space (`SidewinderAssets::eye` from the shipped files).
    const EYE: Vec3 = Vec3::new(0.0001, 6.7223, 1.3465);

    /// A test's head turn: any finite turn is taken as it is, past the head look's limits too; one that is not finite is
    /// refused.
    #[test]
    fn a_test_turn_must_be_finite() {
        assert_eq!(test_turn(Vec2::new(0.5, -0.2)), Some(Vec2::new(0.5, -0.2)));
        assert_eq!(test_turn(Vec2::new(4.0, 2.0)), Some(Vec2::new(4.0, 2.0)));
        for bad in [Vec2::new(f32::NAN, 0.0), Vec2::new(0.0, f32::INFINITY), Vec2::new(f32::NEG_INFINITY, f32::NAN)] {
            assert_eq!(test_turn(bad), None, "{bad:?}");
        }
    }

    /// The seat's camera: at the eye turned by the ship, then the head (yaw about the ship's up, then pitch, no roll).
    #[test]
    fn the_eye_pose_turns_the_ship_then_the_head() {
        let rot = DQuat::from_rotation_y(0.4);
        let (pos, cam) = eye_pose(DVec3::new(1.0, 2.0, 3.0), rot, Vec3::new(0.0, 6.0, 1.0), Vec2::new(0.5, -0.2));
        assert!((pos - (DVec3::new(1.0, 2.0, 3.0) + rot * DVec3::new(0.0, 6.0, 1.0))).length() < 1e-9);
        let fwd = cam * DVec3::NEG_Z;
        let want = DQuat::from_rotation_y(0.9) * DQuat::from_rotation_x(-0.2) * DVec3::NEG_Z;
        assert!((fwd - want).length() < 1e-6, "{fwd:?}");
        // No roll: the view's right stays level.
        assert!((cam * DVec3::X).y.abs() < 1e-6);
    }

    /// The fade lasts the same real time at 30 and at 120 frames a second, and a long frame doesn't swallow it.
    #[test]
    fn the_fade_runs_on_real_seconds() {
        let frames = |fps: f32| {
            let (mut left, mut n) = (FADE_S, 0);
            while left > 0.0 {
                left = fade_step(left, 1.0 / fps);
                n += 1;
            }
            n as f32 / fps
        };
        assert!((frames(30.0) - FADE_S).abs() < 1e-4 && (frames(120.0) - FADE_S).abs() < 1e-4);
        assert!(fade_step(FADE_S, 2.0) > 0.0);
        let seat = Seat { fade: FADE_S / 2.0, ..default() };
        assert!((seat.fade_alpha() - 0.5).abs() < 1e-6);
    }
}
