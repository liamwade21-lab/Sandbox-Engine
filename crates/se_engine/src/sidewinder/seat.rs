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
//!   the ship (the spec's edge cases: "if that was in flight it is lost"). Landed, he is set down beside the ship's left side,
//!   unless something else already put him somewhere (the main menu's Play puts him on Earth).

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::math::{DQuat, Vec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::egui;
use se_core::elite_asset::Binds;

use crate::camera::FlyCamera;
use crate::mods::{ModControl, ModRegistry};
use crate::sandbox::{Mode, Sandbox};
use crate::system::SystemBody;
use crate::universe::SimTime;

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
    /// Head look: on, and the view's turn from straight ahead (radians: yaw to the left, pitch up).
    pub head_look: bool,
    pub look: Vec2,
    /// B was pressed this frame (`seat_camera` acts on it).
    leave: bool,
    regain: u8,
    /// The game clock's rate and pause as they were when he sat down (given back when he gets out).
    clock: Option<(f64, bool)>,
    /// The fade from black after boarding or getting out: real seconds left of it (0: none).
    pub fade: f32,
}

impl Seat {
    /// Just sat down: the game clock to real time and running, its old state kept.
    pub fn boarded(sim: &mut SimTime) -> Seat {
        let clock = Some((sim.rate, sim.paused));
        sim.rate = 1.0;
        sim.paused = false;
        Seat { seated: true, regain: 3, clock, fade: FADE_S, ..default() }
    }

    /// Get out (as B does: only when landed).
    pub fn request_leave(&mut self) {
        self.leave = true;
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

/// The seat's camera at the pilot's eyes (bound to the body this frame), and getting out: B (landed only, `leave_rule`) or the
/// mod off by any way (in flight that loses the ship).
#[allow(clippy::too_many_arguments)]
pub fn seat_camera(
    mut sb: ResMut<Sandbox>,
    mut seat: ResMut<Seat>,
    mut control: ResMut<ModControl>,
    mut ship: ResMut<Sidewinder>,
    assets: Option<Res<SidewinderAssets>>,
    mut cam: ResMut<FlyCamera>,
    bodies: Query<&SystemBody>,
    mut msg: ResMut<crate::messages::Messages>,
    mut pilot: ResMut<Pilot>,
    mut sim: ResMut<SimTime>,
    data: Res<crate::data::SeData>,
) {
    if !seat.seated {
        return;
    }
    let (Some(assets), Some(p)) = (assets, ship.parked.as_ref()) else {
        seat.seated = false;
        return;
    };
    // In hyperspace's tunnel `hyper::tunnel` holds the camera; the mod off there, or the ship destroyed (out of oxygen), loses
    // the ship, and he goes back on Earth.
    if pilot.jump.is_some() {
        if control.pilot && sb.mode == Mode::Pilot && !pilot.destroyed {
            return;
        }
        seat.seated = false;
        seat.head_look = false;
        seat.look = Vec2::ZERO;
        if let Some((rate, paused)) = seat.clock.take() {
            sim.rate = rate;
            sim.paused = paused;
        }
        pilot.jump = None;
        pilot.cruise = None;
        if let Some(mut gone) = ship.parked.take() {
            super::remove_solids(&mut sb, &mut gone);
        }
        control.ship_parked = false;
        if sb.mode == Mode::Pilot {
            crate::sandbox::put_on_earth(&data, sim.jd, &mut cam, &mut sb, "the Sidewinder was lost in hyperspace: back");
        }
        if pilot.destroyed {
            if control.pilot {
                control.request_off();
            }
            msg.show("The Sidewinder was destroyed in hyperspace".to_string());
            info!("sidewinder: destroyed in hyperspace; the ship is lost");
        } else {
            msg.show("The Sidewinder was lost in hyperspace".to_string());
            info!("sidewinder: the mod went off in hyperspace; the ship is lost");
        }
        return;
    }
    let Some(b) = bodies.iter().find(|b| b.obj == p.obj) else { return };
    let eye = Vec3::from_array(assets.eye).as_dvec3();
    let look = DQuat::from_rotation_y(seat.look.x as f64) * DQuat::from_rotation_x(seat.look.y as f64);
    let eye_body = p.pos + p.rot * eye;
    let rot_body = (p.rot * look).normalize();
    // Destroyed (the hull at zero): the ship is lost and he is set down where it was.
    if pilot.destroyed && control.pilot {
        msg.show("The Sidewinder was destroyed".to_string());
        info!("sidewinder: destroyed");
        control.request_off();
    }
    if seat.leave {
        seat.leave = false;
        match leave_rule(pilot.landed, true) {
            Leave::Out => {
                control.request_off();
                info!("sidewinder: B: getting out");
            }
            _ => msg.show("Disembark is only possible when landed".to_string()),
        }
    }
    if control.pilot && sb.mode == Mode::Pilot {
        cam.pos = b.pos.offset_m(b.orient * eye_body);
        cam.rot = (b.orient * rot_body).normalize();
        crate::sandbox::bind_camera(&mut cam, b);
        return;
    }
    if control.pilot {
        // (Boarded this frame: the sandbox leaves first person at the end of its step; the seat takes the camera next frame.)
        return;
    }
    // Out. Beside the ship's left side, on the ground, facing the way the ship faces; unless he's already been put somewhere.
    // Flying, the ship is lost (left alone it would hang in the air as a solid thing: the checker's first finding).
    seat.seated = false;
    seat.head_look = false;
    seat.look = Vec2::ZERO;
    seat.fade = FADE_S;
    if let Some((rate, paused)) = seat.clock.take() {
        sim.rate = rate;
        sim.paused = paused;
    }
    let lost = pilot.destroyed || leave_rule(pilot.landed, false) == Leave::Lost;
    let (obj, pos, rot) = (p.obj, p.pos, p.rot);
    // In supercruise, or by a star or another body with no ground to stand on, there is nowhere beside the ship to set him
    // down (checker 2: he landed on an asteroid millions of km out, or would wait for ever by a star): back on Earth, as the
    // main menu's Play does.
    let far = pilot.cruise.is_some() || bodies.iter().find(|bb| bb.obj == obj).is_none_or(|bb| bb.star);
    pilot.cruise = None;
    pilot.glide = None;
    if lost {
        if let Some(mut gone) = ship.parked.take() {
            super::remove_solids(&mut sb, &mut gone);
        }
        control.ship_parked = false;
    }
    if sb.mode == Mode::Pilot && far {
        crate::sandbox::put_on_earth(&data, sim.jd, &mut cam, &mut sb, "the Sidewinder was lost far from any ground: back");
        msg.show(if pilot.destroyed { "The Sidewinder was destroyed" } else { "The Sidewinder was lost" }.to_string());
    } else if sb.mode == Mode::Pilot {
        let side = Vec3::new(assets.shape.lo.x - EXIT_OUT_M, 1.0, (assets.shape.lo.z + assets.shape.hi.z) * 0.5);
        let point = pos + rot * side.as_dvec3();
        sb.set_down(obj, point, None, eye_body, rot);
        msg.show(if pilot.destroyed { "The Sidewinder was destroyed" } else if lost { "The Sidewinder was lost" } else { "Disembarked" }.to_string());
        info!("sidewinder: out, beside the ship{}", if pilot.destroyed { "; it was destroyed" } else if lost { "; it was flying and is lost" } else { "" });
    } else if lost {
        msg.show("The Sidewinder was lost".to_string());
        info!("sidewinder: the mod went off while flying; the ship is lost");
    }
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
