//! Getting in and out of the Sidewinder and sitting in it (the spec's F2, step 1.3): E at the parked ship boards it, the camera
//! sits at the pilot's eyes with Elite's head look on the middle button, and B (landed only, the spec's D8) sets him down
//! beside it. Both cuts, into the seat and out of it, come up from black over `FADE_S` real seconds (ours, F: Elite's own
//! boarding is Odyssey's walk-in sequence, not built).
//!
//! - Boarding turns the mod on (`ModControl::board`): the sandbox leaves first person at the end of that frame and the seat
//!   writes the camera from then on (`seat_camera`, bound to the body every frame: the 500 m-a-frame rule).
//! - Seated, every key's press is taken from the frame's input (`take_keys`) once Elite's bindings have been read, so the
//!   port's own readers never see them (K would slow the clock, I open the wiki); Esc (the menu) and F11 (a screenshot) are
//!   left. The game clock runs at real time while he sits and is given back when he gets out (the spec's edge cases).
//! - Getting out (`leave_rule`): B only when landed (the spec's D8); the mod turned off by any other way while flying loses
//!   the ship (the spec's edge cases: "if that was in flight it is lost"). Every way out (B, the mod off, the ship destroyed,
//!   lost in hyperspace, removed) goes through one function, `release` (`exit_rule` says where he goes, the spec's F6):
//!   beside the ship's left side when it is landed, on the ground under it when it was flying by ground he can stand on,
//!   else on Earth, unless something else already put him somewhere (the main menu's Play puts him on Earth). The seat's
//!   view stays until the sandbox has set him down.

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

use super::hud_anchor::{LEFT_PANEL, RIGHT_PANEL};
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
    /// The camera is at the pilot's eye this frame (`seat_camera` wrote it, or hyperspace's tunnel did): what `draw` shows
    /// from (the seat's set) and when the view takes Elite's field of view (`seat_fov`).
    pub view: bool,
    /// Head look: on, and the view's turn from straight ahead (radians: yaw to the left, pitch up).
    pub head_look: bool,
    pub look: Vec2,
    /// The head's turn the camera is written with this frame (radians: yaw to the left, pitch up; `head_aim`): head look's
    /// while it is on, else the open side panel's, else none; a test's `sw look` over all. `seat_camera` writes it and the
    /// HUD draws with it (after `seat_camera`: the mod's `plugin` orders them).
    pub head: Vec2,
    /// A test's head turn (`sw look YAW PITCH`; None: the head is the pilot's again).
    pub test_look: Option<Vec2>,
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
}

impl Seat {
    /// Just sat down: the game clock to real time and running, its old state kept.
    pub fn boarded(sim: &mut SimTime) -> Seat {
        let clock = Some((sim.rate, sim.paused));
        sim.rate = 1.0;
        sim.paused = false;
        Seat { seated: true, regain: 3, clock, fade: FADE_S, boarding: true, ..default() }
    }

    /// Get out (as B does: only when landed).
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

/// What getting out does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    /// Refused, with Elite's message (B while flying).
    Refused,
    /// Out beside the ship, which stays parked.
    Out,
    /// Out, and the ship is lost (the mod turned off while flying).
    Lost,
}

/// The rule for getting out (the spec's D8 and its edge cases): `by_b` for B or Disembark, else the mod turned off by any way.
pub fn leave_rule(landed: bool, by_b: bool) -> Leave {
    match (landed, by_b) {
        (true, _) => Leave::Out,
        (false, true) => Leave::Refused,
        (false, false) => Leave::Lost,
    }
}

/// Why the seat is released (every way out of it goes through `release`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// B while landed, or the mod turned off by any way (in flight, or in hyperspace, that loses the ship).
    Out,
    /// The hull at zero or the oxygen run out.
    Destroyed,
    /// The ship removed (`sw remove`, the Mods row) while he sits in it.
    Removed,
}

/// Where he goes out of the seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Beside the ship's left side, on the ground (it is landed).
    Beside,
    /// On the ground under the ship, on its body (it was flying: destroyed, lost or removed in flight).
    Under,
    /// On Earth, as the main menu's Play puts him: there is no ground to stand on by the ship.
    Earth,
    /// Nowhere: the sandbox isn't in the seat's mode (he is already somewhere: the mod went off before it took the camera).
    Stay,
}

/// The release's rule (the spec's F6): whether the ship is lost and where he goes. Beside the ship when it is landed; on the
/// ground under it when it was flying by a body he can stand on (`ground`: a solid body's loaded ground, out of supercruise
/// and hyperspace); else (deep space, the tunnel, a star, a gas giant, ground not loaded, no ship) on Earth.
pub fn exit_rule(why: Why, landed: bool, ground: bool, ship: bool, in_seat_mode: bool) -> (bool, Place) {
    let lost = match why {
        Why::Out => leave_rule(landed, false) == Leave::Lost,
        Why::Destroyed | Why::Removed => true,
    };
    let place = match (in_seat_mode, ship && ground, landed) {
        (false, _, _) => Place::Stay,
        (true, false, _) => Place::Earth,
        (true, true, true) => Place::Beside,
        (true, true, false) => Place::Under,
    };
    (lost, place)
}

/// Whether a boarding is refused this frame (`seat_camera`): only while boarding, before the sandbox first gives the seat
/// its mode, and only from anywhere but on foot (the sandbox gives the seat its mode only from walking: a test's `sw board`
/// in the free flight would leave the free camera where it was). Once seated in the seat's mode, a mode the seat loses is
/// one of the ways out (`release`), never a refused boarding: a refusal there would free the seat with the ship left hanging
/// in the air.
pub fn boarding_refused(boarding: bool, mode: Mode) -> bool {
    boarding && !matches!(mode, Mode::Walk | Mode::Pilot)
}

/// The release's state changes (the pure part of `release`): out of the seat, the game clock given back, head look cleared,
/// the fade from black, the mod asked off if it is the one piloting, and the flight's transient states (hyperspace's tunnel,
/// supercruise, the glide) ended.
fn free_seat(seat: &mut Seat, sim: &mut SimTime, control: &mut ModControl, pilot: &mut Pilot) {
    seat.seated = false;
    seat.boarding = false;
    seat.leave = false;
    seat.remove = false;
    seat.head_look = false;
    seat.look = Vec2::ZERO;
    seat.head = Vec2::ZERO;
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
    if !sb.walking() || !sb.placed || sb.menu || control.main_menu || control.skate || sb.skate.is_some() {
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
            *seat = Seat::boarded(&mut sim);
            *pilot = pilot.boarding();
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
    *pilot_keys = if free { PilotKeys::default() } else { super::pilot::read_keys(&assets.keys, &assets.pad, pad, &keys, &buttons, &motion, real && seat.regain == 0 && !seat.head_look, panels.open.is_some()) };
    if !free {
        if keys.just_pressed(KeyCode::KeyB) {
            seat.leave = true;
        }
        // Head look (`HeadLookToggle`, the middle button in Elite's file).
        let toggle = assets.keys.get("HeadLookToggle").is_some_and(|a| a.keys().any(|k| k.device == "Mouse" && k.key == "Mouse_3"));
        if toggle && buttons.just_pressed(MouseButton::Middle) {
            seat.head_look = !seat.head_look;
            if !seat.head_look {
                seat.look = Vec2::ZERO;
            }
        }
        if seat.head_look && real && seat.regain == 0 {
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

/// The head's turn from straight ahead (`look`: radians, yaw to the left about the ship's up, then pitch up about the turned
/// x; no roll), in the ship's axes. The one function for it: `seat_camera` writes the camera with it and the HUD
/// (`hud_anchor::SeatView`) projects with it.
pub fn head_turn(look: Vec2) -> Quat {
    Quat::from_rotation_y(look.x) * Quat::from_rotation_x(look.y)
}

/// The seat's camera in the frame the ship's pose is given in (`pos`, `rot`: the ship space's origin and turn): at the eye
/// (`eye`, ship space), turned by the ship and then the head (`head_turn(head)`). What `seat_camera` writes.
pub fn eye_pose(pos: DVec3, rot: DQuat, eye: Vec3, head: Vec2) -> (DVec3, DQuat) {
    (pos + rot * eye.as_dvec3(), (rot * head_turn(head).as_dquat()).normalize())
}

/// The head's turn while a side panel is open by its key (radians, yaw left, pitch up): the user's own pictures of Elite with
/// each panel open (`2.webp` the left, `3.webp` the right), fitted from the parts seen in them and in the straight-ahead
/// `1.webp` (the red canopy lamps' directions from the mod's own picture, Elite's HUD parts' from `1.webp` at its measured
/// focal), a turn about the eye as the seat's camera makes it (grade D; `s1_turns.py`, `re/elite/notes/sidewinder_hud.md`):
/// the left 39.28 degrees left and 8.35 down (residuals 6 to 29 px, rms 21, at 2000 px wide), the right 38.17 right and 2.50
/// down (10 to 39 px, rms 27). What is left over says Elite's head also moves as it turns (not built: the seat only turns).
pub const PANEL_TURN_LEFT: Vec2 = Vec2::new(0.6856, -0.1457);
pub const PANEL_TURN_RIGHT: Vec2 = Vec2::new(-0.6662, -0.0436);

/// The screen the panels' turns were measured on (the user's, 3440 x 1440): its aspect.
const MEASURED_ASPECT: f32 = 3440.0 / 1440.0;

/// How far down a side panel its rows reach (in its anchor's -1..1 up from the bottom): `panels::draw` puts the tabs and up to
/// ten rows in the panel's top 0.04 + 10 x 0.022 of its 0.5 shares (52 %), and the selection's margin; under them is only
/// the keys' help line.
const ROWS_DOWN: f32 = -0.2;

/// The horizontal angle (radians, atan(x / -z) in the view turned by `turn`) of the panel's outer edge (the left panel's
/// left, the right panel's right) where its rows are: the outermost of its top corner and its point at the rows' foot.
/// None if one is behind the eye.
fn outer_edge(side: Side, eye: Vec3, turn: Vec2) -> Option<f32> {
    let (anchor, s) = match side {
        Side::Left => (&LEFT_PANEL, -1.0),
        Side::Right => (&RIGHT_PANEL, 1.0),
    };
    let inv = head_turn(turn).inverse();
    let at = |t: f32| {
        let v = inv * (Vec3::from_array(anchor.centre) - eye + s * Vec3::from_array(anchor.right) + t * Vec3::from_array(anchor.up));
        (v.z < -1e-3).then(|| (v.x / -v.z).atan())
    };
    let (top, foot) = (at(1.0)?, at(ROWS_DOWN)?);
    Some(if side == Side::Left { top.min(foot) } else { top.max(foot) })
}

/// Half the view's angle (radians) across at `aspect`, the projection's (`seat_vertical_fov`).
fn half_across(aspect: f32) -> f32 {
    ((0.5 * seat_vertical_fov(aspect)).tan() * aspect).atan()
}

/// The margin (radians) a panel's outer edge keeps from the screen's edge in the measured case (`outer_edge` with the
/// measured turn, on the measured screen, at the game's field of view): the left panel's 1.90 degrees, the right's 0.12.
fn panel_margin(side: Side, eye: Vec3) -> f32 {
    let measured = if side == Side::Left { PANEL_TURN_LEFT } else { PANEL_TURN_RIGHT };
    outer_edge(side, eye, measured).map_or(0.0, |x| half_across(MEASURED_ASPECT) - x.abs())
}

/// The head's turn to an open side panel on a window `aspect` wide. The pitch is always the measured one (D: Elite's own
/// picture cuts the right panel's lower third off, and the mod mustn't show otherwise), and so is the yaw (D) on the
/// measured screen and wherever the panel's outer edge stays on the screen with it. On a narrower window, where that edge
/// (`outer_edge`: where the rows are) would come nearer the screen's edge than its margin in the measured case
/// (`panel_margin`), the yaw grows by just what brings it back to that margin (a stand-in, grade E: Elite's turn at this
/// aspect is not measured; the rule is ours, stated, not tuned). Worked out from the panel's anchor and the window's
/// projection whenever it is asked for: no table of sizes.
pub fn panel_turn(side: Side, eye: Vec3, aspect: f32) -> Vec2 {
    let mut turn = if side == Side::Left { PANEL_TURN_LEFT } else { PANEL_TURN_RIGHT };
    let room = half_across(aspect) - panel_margin(side, eye);
    // (A yaw moves the edge's angle nearly one for one: a few rounds settle it. Turning left, yaw up, moves the view's
    // contents right.)
    for _ in 0..16 {
        let Some(x) = outer_edge(side, eye, turn) else { break };
        let dx = match side {
            Side::Left => (-room - x).max(0.0),
            Side::Right => (room - x).min(0.0),
        };
        if dx.abs() < 1e-7 {
            break;
        }
        turn.x += dx;
    }
    turn
}

/// The head's turn this frame (`Seat::head`): a test's (`sw look`) over all, then head look's while it is on, then the open
/// side panel's (`panel`: `panel_turn`'s), else straight ahead. The turn to a panel and back is at once: how long Elite's
/// takes is not known (a stand-in, grade E: no ease; it is on the measuring list).
pub fn head_aim(test: Option<Vec2>, head_look: bool, look: Vec2, panel: Option<Vec2>) -> Vec2 {
    match (test, head_look, panel) {
        (Some(t), ..) => t,
        (None, true, _) => look,
        (None, false, Some(p)) => p,
        (None, false, None) => Vec2::ZERO,
    }
}

/// The seat's camera at the pilot's eyes (bound to the body this frame), and every way out of the seat (`release`): B
/// (landed only, `leave_rule`), the mod off by any way (in flight that loses the ship), the ship destroyed or removed.
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
    panels: Res<Panels>,
    window: Query<&Window, With<PrimaryWindow>>,
) {
    // (Set again below where the camera is at the eye: every other way through, it isn't there.)
    seat.view = false;
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
        free_seat(&mut seat, &mut sim, &mut control, &mut pilot);
        seat.setting_down = false;
        msg.show("Board the Sidewinder on foot".to_string());
        info!("sidewinder: boarding refused: not on foot");
        return;
    }
    if sb.mode == Mode::Pilot {
        seat.boarding = false;
    }
    // Every way out: the ship removed (or gone), destroyed, the mod turned off by any way, B while landed.
    let why = if seat.remove || ship.parked.is_none() {
        Some(Why::Removed)
    } else if pilot.destroyed {
        Some(Why::Destroyed)
    } else if !control.pilot {
        Some(Why::Out)
    } else if std::mem::take(&mut seat.leave) {
        match leave_rule(pilot.landed, true) {
            Leave::Out => Some(Why::Out),
            _ => {
                msg.show("Disembark is only possible when landed".to_string());
                info!("sidewinder: B refused: disembark is only possible when landed");
                None
            }
        }
    } else {
        None
    };
    if let Some(why) = why {
        let mut out = Out { seat: &mut *seat, sim: &mut *sim, control: &mut *control, pilot: &mut *pilot, ship: &mut *ship, sb: &mut *sb, cam: &mut *cam, msg: &mut *msg };
        release(why, &mut out, &bodies, &data, &assets);
        // (The camera is at the eye this frame: the seat's view until he is set down.)
        seat.view = seat.setting_down && sb.walking() && !sb.placed;
        return;
    }
    // In hyperspace's tunnel `hyper::tunnel` writes the camera at the eye: still the seat's view, Elite's field of view and all.
    if pilot.jump.is_some() {
        seat.view = true;
        return;
    }
    let Some(p) = ship.parked.as_ref() else { return };
    let Some((b, _)) = bodies.iter().find(|(b, _)| b.obj == p.obj) else { return };
    let eye = Vec3::from_array(assets.eye);
    let aspect = window.single().map_or(ELITE_ASPECT, |w| w.width() / w.height().max(1.0));
    seat.head = head_aim(seat.test_look, seat.head_look, seat.look, panels.open.map(|side| panel_turn(side, eye, aspect)));
    let (eye_body, rot_body) = eye_pose(p.pos, p.rot, eye, seat.head);
    if control.pilot && sb.mode == Mode::Pilot {
        cam.pos = b.pos.offset_m(b.orient * eye_body);
        cam.rot = (b.orient * rot_body).normalize();
        crate::sandbox::bind_camera(&mut cam, b);
        seat.view = true;
    }
    // (Otherwise boarded this frame: the sandbox leaves first person at the end of its step; the seat takes the camera next
    // frame.)
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
}

/// Releases the seat, whichever way he leaves it (`Why`): `free_seat`'s state changes; the camera written once more this
/// frame at the eye and bound to the body (the 500 m-a-frame rule: whatever takes it carries it on from there); he is put
/// down beside the landed ship, on the ground under a flying one, else on Earth (`exit_rule`); a lost or removed ship goes
/// (left alone in flight it would hang in the air as a solid thing: the checker's first finding).
fn release(why: Why, out: &mut Out, bodies: &Query<(&SystemBody, Option<&Terrain>)>, data: &crate::data::SeData, assets: &SidewinderAssets) {
    let in_jump = out.pilot.jump.is_some();
    let in_cruise = out.pilot.cruise.is_some();
    let landed = out.pilot.landed;
    let at = out.ship.parked.as_ref().map(|p| (p.obj, p.pos, p.rot));
    let body = at.and_then(|(obj, _, _)| bodies.iter().find(|(b, _)| b.obj == obj));
    // Ground to stand on by the ship: a solid body's (not a star's or a gas giant's), loaded under it and not the sea (the
    // set-down's own test: an ocean world's ground at or under its radius), out of supercruise and hyperspace (checker 2: he
    // landed on an asteroid millions of km out, or would wait for ever by a star). Else the launch's dry-land place.
    let ground = !in_cruise
        && !in_jump
        && body.zip(at).is_some_and(|((b, t), (_, pos, _))| {
            t.filter(|t| !b.star && !t.proc.kind.is_giant()).is_some_and(|t| {
                let (r, level) = crate::ship::ground_on(b.radius_m, b.shape, Some(t), pos);
                let land = (pos / b.shape).try_normalize().map_or(0.0, |dir| r / (dir * b.shape).length() - b.radius_m);
                level.is_some() && !(t.has_ocean && land <= 0.0)
            })
        });
    let (lost, place) = exit_rule(why, landed, ground, at.is_some(), out.sb.mode == Mode::Pilot);
    free_seat(out.seat, out.sim, out.control, out.pilot);
    let eye = Vec3::from_array(assets.eye).as_dvec3();
    match (place, at, body) {
        (Place::Beside | Place::Under, Some((obj, pos, rot)), Some((b, _))) => {
            let eye_body = pos + rot * eye;
            out.cam.pos = b.pos.offset_m(b.orient * eye_body);
            out.cam.rot = (b.orient * rot).normalize();
            crate::sandbox::bind_camera(out.cam, b);
            // Beside: out from the hull's left side; under: the ship's own place (the sandbox drops the point to the ground).
            let point = if place == Place::Beside {
                pos + rot * Vec3::new(assets.shape.lo.x - EXIT_OUT_M, 1.0, (assets.shape.lo.z + assets.shape.hi.z) * 0.5).as_dvec3()
            } else {
                pos
            };
            out.sb.set_down(obj, point, None, eye_body, rot);
        }
        (Place::Beside | Place::Under | Place::Earth, ..) => {
            // (`put_on_earth` writes the camera at the spot it waits at, bound to Earth.)
            crate::sandbox::put_on_earth(data, out.sim.jd, out.cam, out.sb, "out of the Sidewinder far from ground beside it: back");
        }
        (Place::Stay, ..) => {}
    }
    if lost {
        if let Some(mut gone) = out.ship.parked.take() {
            super::remove_solids(out.sb, &mut gone);
        }
    }
    out.control.ship_parked = out.ship.parked.is_some();
    let where_ = if in_jump { " in hyperspace" } else { "" };
    let text = match (why, lost) {
        (Why::Destroyed, _) => format!("The Sidewinder was destroyed{where_}"),
        (Why::Removed, _) => "The Sidewinder was removed".to_string(),
        (Why::Out, true) => format!("The Sidewinder was lost{where_}"),
        (Why::Out, false) => "Disembarked".to_string(),
    };
    out.msg.show(text.clone());
    info!("sidewinder: the seat is released ({why:?}): {text}; {}", match place {
        Place::Beside => "out, beside the ship",
        Place::Under => "out, on the ground under the ship",
        Place::Earth => "back on Earth",
        Place::Stay => "already out of the seat's mode",
    });
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
/// (3440 x 1440: a focal of 1347.05 px), wider vertically on a narrower one (4:3: 70.95 degrees).
pub fn seat_vertical_fov(aspect: f32) -> f32 {
    let k = (ELITE_ASPECT / aspect.max(1e-3)).max(1.0);
    2.0 * ((0.5 * SEAT_FOV_DEG.to_radians()).tan() * k).atan()
}

/// While the camera is at the eye (`Seat::view`), the main camera's vertical field of view is Elite's for the window's
/// aspect (`seat_vertical_fov`): written into its projection after everything else that sets it this frame (the flight's
/// `FlyCamera::fov_deg` by `sync_camera_transform`, then the sandbox's first-person view), before its matrices are made and
/// before the capture views copy it (the mod's `plugin` orders it). The player's own setting (`fov_deg`, saved with the
/// settings) is never touched, so nothing has to be given back and nothing can leak: from the frame the camera leaves the
/// eye (`seat_camera` clears `view` once the sandbox has set him down), whichever way he left, the view's own field of view
/// is drawn again. Both changes are logged.
pub fn seat_fov(seat: Res<Seat>, window: Query<&Window, With<PrimaryWindow>>, mut proj: Query<&mut Projection, With<crate::camera::MainCamera>>, mut was: Local<bool>) {
    let aspect = window.single().map_or(ELITE_ASPECT, |w| w.width() / w.height().max(1.0));
    let fov = seat_vertical_fov(aspect);
    for mut p in &mut proj {
        let Projection::Perspective(p) = p.as_mut() else { continue };
        if seat.view != *was {
            let own = p.fov.to_degrees();
            if seat.view {
                info!("sidewinder: the seat's view: vertical field of view {:.3} degrees at the aspect {aspect:.3} (the view's own was {own:.3})", fov.to_degrees());
            } else {
                info!("sidewinder: out of the seat's view: the view's own field of view again, {own:.3} degrees");
            }
        }
        if seat.view {
            p.fov = fov;
        }
    }
    *was = seat.view;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's C13: getting out works only when landed; B in flight is refused, the mod going off in flight loses the ship.
    #[test]
    fn leaving_works_only_when_landed() {
        assert_eq!(leave_rule(true, true), Leave::Out);
        assert_eq!(leave_rule(true, false), Leave::Out);
        assert_eq!(leave_rule(false, true), Leave::Refused);
        assert_eq!(leave_rule(false, false), Leave::Lost);
    }

    /// The release's rule (F6): out while landed keeps the ship and sets him down beside it; in flight the ship is lost and he
    /// goes to the ground under it, or to Earth with no ground to stand on; destroyed or removed, the ship goes; outside the
    /// seat's mode he stays where he is.
    #[test]
    fn the_release_rule() {
        // (why, landed, ground, ship, in the seat's mode)
        assert_eq!(exit_rule(Why::Out, true, true, true, true), (false, Place::Beside));
        assert_eq!(exit_rule(Why::Out, false, true, true, true), (true, Place::Under));
        assert_eq!(exit_rule(Why::Out, false, false, true, true), (true, Place::Earth));
        assert_eq!(exit_rule(Why::Destroyed, true, true, true, true), (true, Place::Beside));
        assert_eq!(exit_rule(Why::Destroyed, false, true, true, true), (true, Place::Under));
        assert_eq!(exit_rule(Why::Destroyed, false, false, true, true), (true, Place::Earth));
        assert_eq!(exit_rule(Why::Removed, true, true, true, true), (true, Place::Beside));
        assert_eq!(exit_rule(Why::Removed, false, true, true, true), (true, Place::Under));
        assert_eq!(exit_rule(Why::Removed, true, true, false, true), (true, Place::Earth));
        assert_eq!(exit_rule(Why::Out, true, true, true, false), (false, Place::Stay));
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
        assert!(Seat::boarded(&mut sim).boarding);
        // Another mod on in flight: the release's rule loses the ship.
        assert_eq!(exit_rule(Why::Out, false, true, true, false).0, true);
    }

    /// The release's state changes: out of the seat, the old clock given back, head look and a pending B or removal cleared,
    /// the fade started, the flight's transient states ended; the ship's systems kept.
    #[test]
    fn the_release_frees_the_seat() {
        let mut sim = SimTime { jd: 2_460_000.0, rate: 1000.0, paused: true };
        let mut seat = Seat::boarded(&mut sim);
        assert_eq!((sim.rate, sim.paused), (1.0, false));
        seat.head_look = true;
        seat.look = Vec2::new(0.5, -0.2);
        seat.request_leave();
        seat.request_remove();
        let mut control = ModControl::default();
        control.pilot = true;
        control.active = Some(CODE.to_string());
        let mut pilot = Pilot::default();
        pilot.glide = Some(elite_sim::glide::Glide::default());
        pilot.fuel.main = 1.25;
        free_seat(&mut seat, &mut sim, &mut control, &mut pilot);
        assert!(!seat.seated && !seat.head_look && seat.look == Vec2::ZERO && !seat.leave && !seat.remove);
        assert!(seat.setting_down && seat.fade == FADE_S && seat.clock.is_none());
        assert_eq!((sim.rate, sim.paused), (1000.0, true));
        assert!(pilot.glide.is_none() && pilot.cruise.is_none() && pilot.jump.is_none());
        assert_eq!(pilot.fuel.main, 1.25);
        // A second release (nothing kept of the clock) leaves the clock as it is.
        sim.rate = 5.0;
        free_seat(&mut seat, &mut sim, &mut control, &mut pilot);
        assert_eq!(sim.rate, 5.0);
    }

    /// In the seat's view the projection takes Elite's field of view; out of it the seat leaves the view's own (written each
    /// frame by the camera's systems before it) as it is.
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

    /// A panel opened by its key turns the head to it and closing it gives the head back; the pilot's own head look and a
    /// test's turn come first; getting out leaves the head straight.
    #[test]
    fn a_panel_turns_the_head_and_closing_gives_it_back() {
        let own = Vec2::new(0.3, -0.1);
        let turn = |side| panel_turn(side, EYE, 16.0 / 9.0);
        assert_eq!(head_aim(None, false, Vec2::ZERO, None), Vec2::ZERO);
        assert_eq!(head_aim(None, false, Vec2::ZERO, Some(turn(Side::Left))), turn(Side::Left));
        assert_eq!(head_aim(None, true, own, Some(turn(Side::Left))), own);
        assert_eq!(head_aim(Some(Vec2::X), true, own, Some(turn(Side::Left))), Vec2::X);
        // Through the panels' own keys: 1 opens the left, 1 again closes it.
        let mut panels = Panels::default();
        let one = super::super::panels::PanelKeys { left: true, ..default() };
        panels.update(&one, None);
        assert_eq!(head_aim(None, false, Vec2::ZERO, panels.open.map(turn)), turn(Side::Left));
        panels.update(&one, None);
        assert_eq!(head_aim(None, false, Vec2::ZERO, panels.open.map(turn)), Vec2::ZERO);
        // The turns point at their panels: left and down, right and down.
        assert!(turn(Side::Left).x > 0.6 && turn(Side::Right).x < -0.6 && turn(Side::Left).y < 0.0 && turn(Side::Right).y < 0.0);
        let mut sim = SimTime { jd: 2_460_000.0, rate: 1.0, paused: false };
        let mut seat = Seat::boarded(&mut sim);
        seat.head = PANEL_TURN_LEFT;
        free_seat(&mut seat, &mut sim, &mut ModControl::default(), &mut Pilot::default());
        assert_eq!(seat.head, Vec2::ZERO);
    }

    /// The eye: the cockpit's `!Camera` bone in the ship's space (`SidewinderAssets::eye` from the shipped files).
    const EYE: Vec3 = Vec3::new(0.0001, 6.7223, 1.3465);

    /// A panel's turn by the stated rule (`panel_turn`): on the measured screen (and a wider one) both turns are exactly the
    /// measured ones; on narrower windows the pitch is still the measured one and each panel's outer edge, where its rows
    /// are, is inside the screen (projected as the HUD projects it), the yaw grown only as far as that takes.
    #[test]
    fn an_open_panel_keeps_its_outer_edge_on_the_screen() {
        use super::super::hud_anchor::SeatView;
        use bevy_egui::egui;
        assert!((panel_margin(Side::Left, EYE).to_degrees() - 1.90).abs() < 0.01, "{}", panel_margin(Side::Left, EYE).to_degrees());
        for aspect in [MEASURED_ASPECT, 32.0 / 9.0] {
            assert_eq!(panel_turn(Side::Left, EYE, aspect), PANEL_TURN_LEFT);
            assert_eq!(panel_turn(Side::Right, EYE, aspect), PANEL_TURN_RIGHT);
        }
        for (w, h) in [(1600.0f32, 900.0f32), (1920.0, 1080.0), (1024.0, 768.0), (2560.0, 1440.0)] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
            for (side, anchor, measured, s) in [(Side::Left, &LEFT_PANEL, PANEL_TURN_LEFT, -1.0f32), (Side::Right, &RIGHT_PANEL, PANEL_TURN_RIGHT, 1.0)] {
                let turn = panel_turn(side, EYE, w / h);
                assert_eq!(turn.y, measured.y, "{side:?} at {w}x{h}: the pitch");
                assert!(turn.x.abs() > measured.x.abs(), "{side:?} at {w}x{h}: the yaw didn't grow");
                let view = SeatView::new(EYE, turn, screen);
                // The outer edge from the top corner to the rows' foot, and each row's end there.
                for t in [1.0, 0.6, 0.2, ROWS_DOWN] {
                    let p = view.project(Vec3::from_array(anchor.centre) + s * Vec3::from_array(anchor.right) + t * Vec3::from_array(anchor.up)).unwrap();
                    assert!(p.x >= 0.0 && p.x <= w, "{side:?} at {w}x{h} ({s}, {t}): {p:?}");
                }
                // Just enough: the edge sits at the margin, not further in.
                let x = outer_edge(side, EYE, turn).unwrap();
                assert!((half_across(w / h) - x.abs() - panel_margin(side, EYE)).abs() < 1e-4, "{side:?} at {w}x{h}");
            }
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
