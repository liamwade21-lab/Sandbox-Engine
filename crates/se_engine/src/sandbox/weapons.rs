//! The sandbox's weapons: a small set of Garry's Mod's default loadout (`player_sandbox.lua`), run as Half-Life 2 runs them
//! (Source SDK 2013 `weapon_pistol.cpp`, `weapon_smg1.cpp`, `weapon_shotgun.cpp`, `weapon_crowbar.cpp`,
//! `weapon_physcannon.cpp`, `basecombatweapon_shared.cpp`, `hl2mp_gamerules.cpp`'s bullet impulses): what each does, when it
//! may fire again (its view model's own sequence lengths, as `SequenceDuration`), how it kicks the view, and the sounds it
//! names (`audio.rs`, from the game's sound scripts). Bullets are rays in the sandbox's world: they push props and kick up
//! dust. The gravity gun picks props up and punts them; the physics gun holds one at any distance, turns it (E) and freezes
//! it (right button).
//!
//! The weapon in his hands is Half-Life 2's first-person model with its hands and its animations (`viewmodel.rs`). Without
//! the game (or if a model can't be read) it is the world model, or a plain stand-in, held and moved by code. The HUD is
//! Half-Life 2's: health and ammunition in its yellow on dark panels, the weapon selection across the top.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy_egui::{EguiContexts, egui};

use crate::camera::{FlyCamera, MainCamera};
use crate::system::SystemBody;

use super::assets::{Gpu, GmodAssets};
use super::audio::{Cue, bullet_impact, met_surface};
use super::light::{PropLight, PropMaterial};
use super::movement::UNIT_M;
use super::viewmodel::{self, Sway, ViewMaterial, ViewModel, ViewModelPart};
use super::world::{Met, SandboxWorld};
use super::{Sandbox, WalkInput};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Crowbar,
    GravityGun,
    PhysGun,
    Pistol,
    Smg,
    Shotgun,
}

pub struct WeaponDef {
    pub kind: Kind,
    pub name: &'static str,
    /// Its number key (Half-Life 2's buckets).
    pub slot: u8,
    pub hint: &'static str,
    /// Half-Life 2's first-person model.
    pub view: &'static str,
    /// The world model held in his hands when the view model can't be had, and a plain stand-in without the game.
    model: &'static str,
    stand_in: &'static str,
    /// The world model's forward is its -y (the gravity gun's), not its +y.
    backwards: bool,
    /// Where the world model's origin is held: right, up, forward of the eyes (m), and how big it is drawn.
    hold: Vec3,
    scale: f32,
    /// Rounds in a full clip (0: it has none), and carried at the start (Garry's Mod's loadout).
    clip: u32,
    reserve: u32,
}

impl WeaponDef {
    /// The sequences of its view model the code plays (a test checks the model has each).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn sequences(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Crowbar => &["idle01", "draw", "misscenter1", "misscenter2", "hitcenter1", "hitcenter2", "hitcenter3"],
            Kind::GravityGun => &["idle", "hold_idle", "draw", "fire", "altfire"],
            Kind::PhysGun => &["idle", "hold_idle", "draw"],
            Kind::Pistol => &["idle01", "idle01empty", "draw", "fire", "fire1", "fire2", "fire3", "fireempty", "reload"],
            Kind::Smg => &["idle01", "draw", "fire01", "fire02", "fire03", "fire04", "reload", "dryfire"],
            Kind::Shotgun => &["idle01", "draw", "fire01", "altfire", "pump", "reload1", "reload2", "reload3", "dryfire"],
        }
    }
}

pub const WEAPONS: [WeaponDef; 6] = [
    WeaponDef { kind: Kind::Crowbar, name: "Crowbar", slot: 1, hint: "Left: swing", view: "models/weapons/v_crowbar.mdl", model: "models/weapons/w_crowbar.mdl", stand_in: "placeholder/bar", backwards: false, hold: Vec3::new(0.21, -0.27, 0.42), scale: 0.8, clip: 0, reserve: 0 },
    WeaponDef { kind: Kind::GravityGun, name: "Gravity Gun", slot: 1, hint: "Left: punt   Right: pick up / drop", view: "models/weapons/v_physcannon.mdl", model: "models/weapons/w_physics.mdl", stand_in: "placeholder/gun", backwards: true, hold: Vec3::new(0.22, -0.25, 0.40), scale: 0.72, clip: 0, reserve: 0 },
    WeaponDef { kind: Kind::PhysGun, name: "Physics Gun", slot: 1, hint: "Hold left: carry   Wheel: distance\nE: turn   Right: freeze   R: unfreeze", view: "models/weapons/v_superphyscannon.mdl", model: "models/weapons/w_physics.mdl", stand_in: "placeholder/gun", backwards: true, hold: Vec3::new(0.22, -0.25, 0.40), scale: 0.72, clip: 0, reserve: 0 },
    WeaponDef { kind: Kind::Pistol, name: "9mm Pistol", slot: 2, hint: "Left: fire   R: reload", view: "models/weapons/v_pistol.mdl", model: "models/weapons/w_pistol.mdl", stand_in: "placeholder/gun", backwards: false, hold: Vec3::new(0.15, -0.15, 0.50), scale: 0.85, clip: 18, reserve: 256 },
    WeaponDef { kind: Kind::Smg, name: "SMG", slot: 3, hint: "Hold left: fire   R: reload", view: "models/weapons/v_smg1.mdl", model: "models/weapons/w_smg1.mdl", stand_in: "placeholder/gun", backwards: false, hold: Vec3::new(0.16, -0.19, 0.45), scale: 0.8, clip: 45, reserve: 256 },
    WeaponDef { kind: Kind::Shotgun, name: "Shotgun", slot: 4, hint: "Left: fire   Right: both barrels\nR: reload", view: "models/weapons/v_shotgun.mdl", model: "models/weapons/w_shotgun.mdl", stand_in: "placeholder/gun", backwards: false, hold: Vec3::new(0.16, -0.19, 0.50), scale: 0.8, clip: 6, reserve: 64 },
];

/// Half-Life 2's numbers. The times marked "its sequence" are used only when the view model can't be had: with it, the
/// sequence's own length is what the SDK's `SequenceDuration` gives.
mod num {
    /// `physcannon_tracelength`, `physcannon_maxmass`, units and kg.
    pub const CANNON_REACH: f32 = 250.0;
    pub const CANNON_MAX_MASS: f32 = 250.0;
    /// `physcannon_cone`: the cosine of how far off the cross a prop may be.
    pub const CANNON_CONE: f32 = 0.97;
    /// `physcannon_maxforce`: a launched prop's speed, units/s (less over 100 kg).
    pub const CANNON_LAUNCH: f32 = 1500.0;
    /// A punt's impulse, kg in/s (`PuntVPhysics`' 15000 at the centre).
    pub const CANNON_PUNT: f32 = 15_000.0;
    /// `BULLET_IMPULSE(200, 1225)` for the pistol and the SMG, `(400, 1200)` a pellet: kg in/s.
    pub const BULLET_IMPULSE: f32 = 666.6;
    pub const PELLET_IMPULSE: f32 = 1306.0;
    /// The crowbar: 10 damage's `CalculateMeleeDamageForce`, kg in/s; its reach in units and its refire.
    pub const CROWBAR_IMPULSE: f32 = 3000.0;
    pub const CROWBAR_REACH: f32 = 75.0;
    pub const CROWBAR_REFIRE: f32 = 0.4;
    /// `VECTOR_CONE_xDEGREES`: the sine of half the cone.
    pub const CONE_1: f32 = 0.008_73;
    pub const CONE_5: f32 = 0.043_62;
    pub const CONE_6: f32 = 0.052_34;
    pub const CONE_10: f32 = 0.087_16;
    /// A bullet's reach, units (`MAX_TRACE_LENGTH` is far more; the ground field ends at 60 m).
    pub const BULLET_REACH: f32 = 4096.0;
    /// The pistol: `GetFireRate` held down, `PISTOL_FASTEST_REFIRE_TIME` clicked, `PISTOL_FASTEST_DRY_REFIRE_TIME`; its
    /// accuracy's penalty a shot and at most (`PISTOL_ACCURACY_*`, seconds).
    pub const PISTOL_HELD_REFIRE: f32 = 0.5;
    pub const PISTOL_REFIRE: f32 = 0.1;
    pub const PISTOL_DRY_REFIRE: f32 = 0.2;
    pub const PISTOL_PENALTY: f32 = 0.2;
    pub const PISTOL_PENALTY_MOST: f32 = 1.5;
    pub const SMG_REFIRE: f32 = 0.075;
    /// Its sequences: `reload` 43 frames at 30, `reload` 45.
    pub const PISTOL_RELOAD: f32 = 1.433;
    pub const SMG_RELOAD: f32 = 1.5;
    /// The shotgun's sequences: `fire01`, `altfire`, `pump`, `reload1`, `reload2`, `reload3`, `dryfire`.
    pub const SHOTGUN_FIRE: f32 = 0.333;
    pub const SHOTGUN_DOUBLE: f32 = 0.5;
    pub const SHOTGUN_PUMP: f32 = 0.533;
    pub const SHOTGUN_RELOAD_START: f32 = 0.5;
    pub const SHOTGUN_SHELL: f32 = 0.4;
    pub const SHOTGUN_RELOAD_END: f32 = 0.433;
    pub const DRY_FIRE: f32 = 0.35;
    /// Its `draw` sequence: no shot until it is up (`DefaultDeploy`).
    pub const DEPLOY: f32 = 0.8;
    /// `+use` on a prop: picked up in his hands if it weighs no more than this (kg, `PLAYER_MAX_LIFT_MASS`) and is within
    /// reach (units, `PLAYER_USE_RADIUS`); thrown at `player_throwforce` (kg in/s).
    pub const HAND_MAX_MASS: f32 = 35.0;
    pub const HAND_REACH: f32 = 80.0;
    pub const HAND_THROW: f32 = 1000.0;
    /// The physics gun: metres a wheel notch, and its reach.
    pub const PHYS_WHEEL: f32 = 0.35;
    pub const PHYS_REACH: f32 = 70.0;
}

/// Something seen for a moment: where a bullet struck, the flash at the muzzle, the gravity gun's white flash.
#[derive(Clone, Copy)]
struct Fx {
    kind: FxKind,
    /// Local frame, metres.
    at: Vec3,
    age: f32,
    life: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum FxKind {
    Dust,
    Spark,
    Flash,
    /// `UTIL_ScreenFade` white, alpha 32, a tenth of a second: the gravity gun's launch.
    White,
}

#[derive(Resource)]
pub struct Arsenal {
    pub current: usize,
    clips: [u32; 6],
    reserve: [u32; 6],
    /// The weapons' clock, seconds.
    time: f32,
    /// `m_flNextPrimaryAttack`, and the player's `m_flNextAttack`: until then the weapon does nothing at all (it is
    /// coming up, a clip is going in, the shotgun is being pumped).
    next_fire: f32,
    busy_until: f32,
    /// A clip is going in; when it is in.
    reload_until: Option<f32>,
    /// The pistol: `m_flSoonestPrimaryAttack`, its accuracy's penalty (seconds), when it last fired.
    soonest: f32,
    penalty: f32,
    last_attack: f32,
    /// Shots in this run of fire (`m_nNumShotsFired`, `m_nShotsFired`: which fire animation plays), and how long the
    /// trigger has been held (`m_fFireDuration`: the SMG's kick grows).
    shots: u32,
    fire_time: f32,
    /// The shotgun: loading shells (`m_bInReload`), wanting a pump, a shot asked for while loading.
    in_reload: bool,
    need_pump: bool,
    delayed: [bool; 2],
    /// When the empty click may sound again (`m_flNextEmptySoundTime`).
    next_empty: f32,
    /// The gravity gun's claws are open, and when they may close (`m_flElementDebounce`).
    claws: bool,
    claws_close_at: f32,
    /// How far open they are, 0..1 (`m_ElementParameter`: open in a fifth of a second, shut in half a second).
    claws_pos: f32,
    /// The weapon highlighted in the selection, and when it closes.
    selecting: Option<(usize, f32)>,
    /// Seconds since this weapon came up; the stand-in's kick (dies away) and the crowbar's swing (1 at the start).
    deployed: f32,
    kick: f32,
    swing: f32,
    /// The stand-in's sway: the view's turn last frame, its lag, its bob.
    last_yaw: f32,
    last_pitch: f32,
    lag: Vec2,
    bob: f32,
    /// The view model's sway.
    sway: Sway,
    /// How far off the gravity or physics gun holds its prop (m), and how long it has been out of reach.
    hold_dist: f32,
    astray: f32,
    fx: Vec<Fx>,
    pub health: i32,
    /// The view models, by weapon, once the scene has them; whether each was looked for; whether the weapon in hand has
    /// been brought up (its draw played).
    view: [Option<ViewModel>; 6],
    view_made: [bool; 6],
    drawn: bool,
    /// The stand-in world models' meshes in the scene, by weapon.
    parts: [Vec<Entity>; 6],
    made: [bool; 6],
    /// This frame's picture of things, for the HUD: where the muzzle, the held prop and the effects are in the render frame.
    muzzle: Option<Vec3>,
    /// The view model's triangles as the screen has it, while a gun holds something: the HUD draws its beam over the picture,
    /// so it leaves out what lies over the gun (the gun is in front of its own beam).
    gun_tris: Vec<[Vec3; 3]>,
    beam_to: Option<Vec3>,
    fx_render: Vec<(Fx, Vec3)>,
    hint_until: f32,
}

impl Default for Arsenal {
    fn default() -> Self {
        Arsenal {
            // Garry's Mod puts the physics gun in a new player's hands.
            current: 2,
            clips: WEAPONS.each_ref().map(|w| w.clip),
            reserve: WEAPONS.each_ref().map(|w| w.reserve),
            time: 0.0,
            next_fire: 0.0,
            busy_until: 0.0,
            reload_until: None,
            soonest: 0.0,
            penalty: 0.0,
            last_attack: -10.0,
            shots: 0,
            fire_time: 0.0,
            in_reload: false,
            need_pump: false,
            delayed: [false; 2],
            next_empty: 0.0,
            claws: false,
            claws_close_at: 0.0,
            claws_pos: 0.0,
            selecting: None,
            deployed: 0.0,
            kick: 0.0,
            swing: 0.0,
            last_yaw: 0.0,
            last_pitch: 0.0,
            lag: Vec2::ZERO,
            bob: 0.0,
            sway: Sway::default(),
            hold_dist: 2.0,
            astray: 0.0,
            fx: Vec::new(),
            health: 100,
            view: Default::default(),
            view_made: [false; 6],
            drawn: false,
            parts: Default::default(),
            made: [false; 6],
            muzzle: None,
            gun_tris: Vec::new(),
            beam_to: None,
            fx_render: Vec::new(),
            hint_until: 25.0,
        }
    }
}

impl Arsenal {
    /// Take a weapon in hand.
    pub fn take(&mut self, i: usize) {
        if i >= WEAPONS.len() || i == self.current {
            return;
        }
        self.current = i;
        self.bring_up();
    }

    /// `DefaultDeploy`: the draw plays, and nothing fires until it is up.
    fn bring_up(&mut self) {
        self.deployed = 0.0;
        self.reload_until = None;
        self.in_reload = false;
        self.need_pump = false;
        self.delayed = [false; 2];
        self.shots = 0;
        self.fire_time = 0.0;
        self.swing = 0.0;
        self.claws = false;
        if let Some(v) = self.view[self.current].as_mut() {
            v.set_idle(&["idle01", "idle"]);
        }
        self.next_fire = self.time + self.anim(&["draw"], num::DEPLOY);
        self.busy_until = self.next_fire;
        self.drawn = true;
    }

    fn def(&self) -> &'static WeaponDef {
        &WEAPONS[self.current]
    }

    /// Start one of these sequences on the weapon in hand; how long it lasts (`fallback` without its view model).
    fn anim(&mut self, names: &[&str], fallback: f32) -> f32 {
        self.view[self.current].as_mut().and_then(|v| v.play(names)).unwrap_or(fallback)
    }

    /// The loop the weapon in hand rests in.
    fn idle(&mut self, names: &[&str]) {
        if let Some(v) = self.view[self.current].as_mut() {
            v.set_idle(names);
        }
    }

    /// What the view model is doing, for the log.
    pub fn describe(&self) -> String {
        let def = self.def();
        format!(
            "{} ({}, clip {} of {}, {} carried), view model {}",
            def.name,
            match self.view[self.current].as_ref() {
                Some(v) => format!("playing {}", v.playing()),
                None => "the stand-in".to_string(),
            },
            self.clips[self.current],
            def.clip,
            self.reserve[self.current],
            if self.view[self.current].is_some() { def.view } else { "none" }
        )
    }
}

/// A shot's direction within a cone round `forward` (`cone` the sine of half its angle), as `FireBullets` spreads them:
/// two numbers each the sum of two in -0.5..0.5, taken again until they fall in the unit disc, so most land near the middle.
fn spread(rng: &mut super::audio::Rng, forward: Vec3, cone: f32) -> Vec3 {
    let right = forward.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(forward);
    let (mut x, mut y);
    loop {
        x = rng.range(-0.5, 0.5) + rng.range(-0.5, 0.5);
        y = rng.range(-0.5, 0.5) + rng.range(-0.5, 0.5);
        if x * x + y * y <= 1.0 {
            break;
        }
    }
    (forward + right * (x * cone) + up * (y * cone)).normalize()
}

/// The prop the gravity gun would take: the one under the cross, else the nearest within its cone and reach.
fn cannon_target(world: &SandboxWorld, eye: Vec3, forward: Vec3) -> Option<u32> {
    let reach = num::CANNON_REACH * UNIT_M;
    if let Some(hit) = world.ray(eye, forward, reach, None) {
        if let Met::Prop(id) = hit.met {
            return Some(id);
        }
    }
    world
        .props()
        .filter_map(|p| {
            let to = p.now.pos - eye;
            let d = to.length();
            (d < reach + p.radius && d > 0.01 && (to / d).dot(forward) > num::CANNON_CONE).then_some((p.id, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|p| p.0)
}

/// One frame of the weapon in hand: the keys, the shots, what is held.
pub fn weapon_frame(time: Res<Time<Real>>, input: Res<WalkInput>, assets: Res<GmodAssets>, mut sb: ResMut<Sandbox>, mut ar: ResMut<Arsenal>) {
    let sb = &mut *sb;
    let ar = &mut *ar;
    let dt = time.delta_secs().min(0.1);
    ar.time += dt;
    ar.deployed += dt;
    ar.kick = (ar.kick - dt * 6.0).max(0.0) * (-dt * 9.0).exp();
    ar.swing = (ar.swing - dt / 0.28).max(0.0);
    ar.fx.retain_mut(|f| {
        f.age += dt;
        f.age < f.life
    });
    sb.loop_sound = None;
    sb.rotating = false;
    // A fall too hard hurts: Garry's Mod's sandbox takes a flat 10 (`mp_falldamage 0`).
    for ev in &sb.events {
        if let super::Event::Land(speed) = ev {
            if *speed > super::movement::SAFE_FALL_SPEED {
                ar.health -= 10;
                if ar.health <= 0 {
                    ar.health = 100;
                }
            }
        }
    }
    let on_foot = sb.walking() && sb.placed;
    let menu_open = sb.menu;
    let walking = on_foot && !menu_open;
    let (eye, forward) = (sb.eye, sb.forward);
    let ground = sb.ground_material;
    let Some(world) = sb.world.as_mut() else { return };
    // What is held goes on being held, or is let go when he can't hold it.
    let kind = ar.def().kind;
    if let Some(id) = sb.held {
        let holds = walking && (sb.by_hand || matches!(kind, Kind::GravityGun | Kind::PhysGun)) && world.prop(id).is_some();
        if !holds {
            world.set_held(id, false);
            sb.held = None;
            sb.by_hand = false;
        }
    }
    if !on_foot {
        return;
    }
    if !ar.drawn {
        ar.bring_up();
    }
    // E: a light prop is picked up in his hands and carried before him (Half-Life 2's `+use`); E again puts it down, the
    // left button throws it. The weapon is put away meanwhile.
    if !menu_open {
        if let (Some(id), true) = (sb.held, sb.by_hand) {
            if let Some((com, _)) = world.motion(id) {
                let radius = world.prop(id).map_or(0.3, |p| p.radius);
                let target = eye + forward * (0.55 + radius * 0.9);
                let to = target - com;
                world.set_velocity(id, (to * 16.0).clamp_length_max(12.0), None);
                ar.astray = if to.length() > 1.2 { ar.astray + dt } else { 0.0 };
                if input.use_pressed || input.attack_pressed || ar.astray > 0.4 {
                    world.set_held(id, false);
                    sb.held = None;
                    sb.by_hand = false;
                    ar.astray = 0.0;
                    if input.attack_pressed {
                        let mass = world.prop(id).map_or(1.0, |p| p.mass);
                        world.set_velocity(id, forward * (num::HAND_THROW * 0.0254 / mass).min(12.0), None);
                    }
                    // The weapon comes back up.
                    ar.bring_up();
                }
            }
            return;
        } else if input.use_pressed && sb.held.is_none() {
            let hit = world.ray(eye, forward, num::HAND_REACH * UNIT_M, None).and_then(|h| if let Met::Prop(id) = h.met { Some(id) } else { None });
            match hit.and_then(|id| world.prop(id).map(|p| (id, p.mass, p.frozen))) {
                Some((id, mass, frozen)) if mass <= num::HAND_MAX_MASS && !frozen => {
                    world.set_held(id, true);
                    sb.held = Some(id);
                    sb.by_hand = true;
                    ar.astray = 0.0;
                    return;
                }
                _ => sb.cues.push(Cue::named("HL2Player.UseDeny")),
            }
        }
    }
    // The selection: a number steps through its slot's weapons, the wheel through them all; the left button takes it.
    let now = ar.time;
    let step = input.slot.map(|s| {
        let from = ar.selecting.map_or(ar.current, |s| s.0);
        let in_slot: Vec<usize> = (0..WEAPONS.len()).filter(|i| WEAPONS[*i].slot == s).collect();
        in_slot.iter().copied().find(|i| *i > from && WEAPONS[from].slot == s).or(in_slot.first().copied())
    });
    let wheel = if matches!(kind, Kind::PhysGun) && sb.held.is_some() { 0.0 } else { input.wheel };
    let pick = match step {
        Some(p) => p,
        None if wheel != 0.0 => {
            let from = ar.selecting.map_or(ar.current, |s| s.0) as i32;
            // (`invprev` on the wheel up, `invnext` down.)
            Some((from + if wheel > 0.0 { -1 } else { 1 }).rem_euclid(WEAPONS.len() as i32) as usize)
        }
        None => None,
    };
    if let Some(i) = pick {
        ar.selecting = Some((i, now + 2.5));
        sb.cues.push(Cue::named("Player.WeaponSelectionMoveSlot"));
    } else if input.slot.is_some() {
        sb.cues.push(Cue::named("Player.DenyWeaponSelection"));
    }
    let mut attack_pressed = input.attack_pressed;
    if let Some((i, until)) = ar.selecting {
        if attack_pressed && !menu_open {
            ar.selecting = None;
            ar.take(i);
            info!("sandbox: took the {}", WEAPONS[i].name);
            sb.cues.push(Cue::named("Player.WeaponSelected"));
            attack_pressed = false;
        } else if now > until || input.attack2_pressed {
            ar.selecting = None;
            sb.cues.push(Cue::named("Player.WeaponSelectionClose"));
        }
    }
    if menu_open || ar.selecting.is_some() {
        return;
    }
    let def = ar.def();
    let w = ar.current;
    let rng = &mut sb.rng;
    if now < ar.busy_until {
        return;
    }
    // (`m_fFireDuration`: how long the trigger has been down.)
    ar.fire_time = if input.attack { ar.fire_time + dt } else { 0.0 };

    // A shot along a direction: what it meets takes its push, sounds as what it is made of, and throws dust or a spark.
    let shoot = |world: &mut SandboxWorld, cues: &mut Vec<Cue>, fx: &mut Vec<Fx>, dir: Vec3, reach: f32, impulse: f32, heard: bool| {
        let Some(hit) = world.ray(eye, dir, reach * UNIT_M, None) else { return None };
        if let Met::Prop(id) = hit.met {
            world.impulse(id, dir * (impulse * 0.0254), hit.point);
        }
        if heard {
            cues.push(bullet_impact(&assets, &met_surface(Some(&*world), ground, hit.met), hit.point));
        }
        // (A prop sparks; the ground, a wall and a trunk throw dust.)
        let spark = matches!(hit.met, Met::Prop(_));
        fx.push(Fx { kind: if spark { FxKind::Spark } else { FxKind::Dust }, at: hit.point + hit.normal * 0.02, age: 0.0, life: if spark { 0.12 } else { 0.4 } });
        Some(hit)
    };

    match def.kind {
        Kind::Crowbar => {
            // `CBaseHLBludgeonWeapon::Swing`.
            if input.attack && now >= ar.next_fire {
                ar.next_fire = now + num::CROWBAR_REFIRE;
                ar.swing = 1.0;
                sb.cues.push(Cue::named("Weapon_Crowbar.Single"));
                match shoot(world, &mut sb.cues, &mut ar.fx, forward, num::CROWBAR_REACH, num::CROWBAR_IMPULSE, true) {
                    Some(hit) => {
                        // (`AddViewKick` in `Hit`: pitch 1..2 down, yaw 1..2 to the right.)
                        sb.punch.kick(Vec3::new(rng.range(1.0, 2.0), rng.range(-2.0, -1.0), 0.0));
                        sb.cues.push(Cue::named("Weapon_Crowbar.Melee_HitWorld").at(hit.point));
                        let n = ["hitcenter1", "hitcenter2", "hitcenter3"][rng.pick(3)];
                        ar.anim(&[n, "hitcenter1"], 0.5);
                    }
                    None => {
                        let n = ["misscenter1", "misscenter2"][rng.pick(2)];
                        ar.anim(&[n, "misscenter1"], 0.67);
                    }
                }
            }
        }
        Kind::Pistol | Kind::Smg => {
            let smg = def.kind == Kind::Smg;
            // `FinishReload`.
            if let Some(t) = ar.reload_until {
                if now >= t {
                    let take = (def.clip - ar.clips[w]).min(ar.reserve[w]);
                    ar.clips[w] += take;
                    ar.reserve[w] -= take;
                    ar.reload_until = None;
                }
            }
            // `DefaultReload`: the clip goes in over the reload sequence's length.
            let reload = |ar: &mut Arsenal, cues: &mut Vec<Cue>| {
                if ar.reload_until.is_none() && ar.clips[w] < def.clip && ar.reserve[w] > 0 {
                    let d = ar.anim(&["reload"], if smg { num::SMG_RELOAD } else { num::PISTOL_RELOAD });
                    ar.reload_until = Some(now + d);
                    ar.next_fire = now + d;
                    ar.busy_until = now + d;
                    ar.shots = 0;
                    cues.push(Cue::named(if smg { "Weapon_SMG1.Reload" } else { "Weapon_Pistol.Reload" }));
                }
            };
            if input.reload {
                reload(ar, &mut sb.cues);
            }
            if !smg {
                // `UpdatePenaltyTime`, and `ItemPostFrame`: as fast as he can click, every half second held.
                if !input.attack && ar.soonest < now {
                    ar.penalty = (ar.penalty - dt).clamp(0.0, num::PISTOL_PENALTY_MOST);
                    if ar.reload_until.is_none() {
                        ar.next_fire = ar.next_fire.min(now - 0.1);
                    }
                }
                ar.idle(&[if ar.clips[w] == 0 { "idle01empty" } else { "idle01" }]);
            } else if !input.attack {
                ar.shots = 0;
            }
            if ar.reload_until.is_none() && now >= ar.next_fire {
                if ar.clips[w] == 0 {
                    if ar.reserve[w] > 0 {
                        // (`HandleFireOnEmpty`, then `ReloadOrSwitchWeapons`: a click if he pulled it, and the reload.)
                        if attack_pressed && now >= ar.next_empty {
                            sb.cues.push(Cue::named(if smg { "Weapon_SMG1.Empty" } else { "Weapon_Pistol.Empty" }));
                            ar.next_empty = now + 0.5;
                        }
                        reload(ar, &mut sb.cues);
                    } else if input.attack && (attack_pressed || now >= ar.next_empty) {
                        // `DryFire`.
                        sb.cues.push(Cue::named(if smg { "Weapon_SMG1.Empty" } else { "Weapon_Pistol.Empty" }));
                        ar.next_empty = now + 0.5;
                        ar.soonest = now + num::PISTOL_DRY_REFIRE;
                        ar.next_fire = now + ar.anim(&[if smg { "dryfire" } else { "fireempty" }], num::DRY_FIRE);
                    }
                } else if input.attack {
                    // `PrimaryAttack`.
                    ar.clips[w] -= 1;
                    ar.kick = 1.0;
                    let cone = if smg {
                        ar.shots += 1;
                        ar.next_fire = now + num::SMG_REFIRE;
                        num::CONE_5
                    } else {
                        ar.shots = if now - ar.last_attack > 0.5 { 0 } else { ar.shots + 1 };
                        ar.last_attack = now;
                        ar.soonest = now + num::PISTOL_REFIRE;
                        ar.next_fire = now + num::PISTOL_HELD_REFIRE;
                        // (Each shot starts the view's kick afresh, so quick fire doesn't walk the aim off.)
                        sb.punch.reset(0.0);
                        let ramp = (ar.penalty / num::PISTOL_PENALTY_MOST).clamp(0.0, 1.0);
                        num::CONE_1 + (num::CONE_6 - num::CONE_1) * ramp
                    };
                    // (`GetPrimaryAttackActivity`: the recoil grows with the run of shots.)
                    let names: &[&str] = match (smg, ar.shots) {
                        (false, 0) => &["fire"],
                        (false, 1) => &["fire1", "fire"],
                        (false, 2) => &["fire2", "fire"],
                        (false, _) => &["fire3", "fire"],
                        (true, 0..=1) => &["fire01"],
                        (true, 2) => &["fire02", "fire01"],
                        (true, 3) => &["fire03", "fire01"],
                        (true, _) => &["fire04", "fire01"],
                    };
                    ar.anim(names, 0.4);
                    sb.cues.push(Cue::named(if smg { "Weapon_SMG1.Single" } else { "Weapon_Pistol.Single" }));
                    let dir = spread(rng, forward, cone);
                    shoot(world, &mut sb.cues, &mut ar.fx, dir, num::BULLET_REACH, num::BULLET_IMPULSE, true);
                    ar.fx.push(Fx { kind: FxKind::Flash, at: Vec3::ZERO, age: 0.0, life: 0.05 });
                    if smg {
                        // `DoMachineGunKick( 0.5, 1.0, m_fFireDuration, 2.0 )`: grows over two seconds of fire.
                        let k = ar.fire_time.min(2.0) / 2.0;
                        sb.punch.reset(10.0);
                        let mut kick = Vec3::new(-(0.2 + k), -(0.2 + k) / 3.0, 0.1 + k / 8.0);
                        if rng.pick(3) != 0 {
                            kick.y = -kick.y;
                        }
                        if rng.pick(3) != 0 {
                            kick.z = -kick.z;
                        }
                        let kick = sb.punch.clipped(kick, Vec3::new(24.0, 3.0, 1.0));
                        sb.punch.kick(kick * 0.5);
                    } else {
                        // (`AddViewKick`: the pistol's dips the view a little, to either side.)
                        sb.punch.kick(Vec3::new(rng.range(0.25, 0.5), rng.range(-0.6, 0.6), 0.0));
                        ar.penalty += num::PISTOL_PENALTY;
                    }
                }
            }
        }
        Kind::Shotgun => {
            // `CWeaponShotgun::ItemPostFrame`.
            let dry = |ar: &mut Arsenal, cues: &mut Vec<Cue>| {
                cues.push(Cue::named("Weapon_Shotgun.Empty"));
                ar.next_fire = now + ar.anim(&["dryfire"], num::DRY_FIRE);
            };
            let start_reload = |ar: &mut Arsenal| {
                if ar.reserve[w] == 0 || ar.clips[w] >= def.clip {
                    return;
                }
                if ar.clips[w] == 0 {
                    ar.need_pump = true;
                }
                ar.next_fire = now + ar.anim(&["reload1"], num::SHOTGUN_RELOAD_START);
                ar.in_reload = true;
            };
            let mut done = false;
            if ar.in_reload {
                if input.attack && ar.clips[w] >= 1 {
                    ar.in_reload = false;
                    ar.need_pump = false;
                    ar.delayed[0] = true;
                } else if input.attack2 && ar.clips[w] >= 2 {
                    ar.in_reload = false;
                    ar.need_pump = false;
                    ar.delayed[1] = true;
                } else if now >= ar.next_fire {
                    if ar.reserve[w] > 0 && ar.clips[w] < def.clip {
                        // `Reload`: one shell.
                        ar.clips[w] += 1;
                        ar.reserve[w] -= 1;
                        sb.cues.push(Cue::named("Weapon_Shotgun.Reload"));
                        ar.next_fire = now + ar.anim(&["reload2"], num::SHOTGUN_SHELL);
                    } else {
                        // `FinishReload`.
                        ar.in_reload = false;
                        ar.next_fire = now + ar.anim(&["reload3"], num::SHOTGUN_RELOAD_END);
                    }
                    done = true;
                }
            }
            if !done && ar.need_pump && now >= ar.next_fire {
                // `Pump`.
                ar.need_pump = false;
                sb.cues.push(Cue::named("Weapon_Shotgun.Special1"));
                ar.next_fire = now + ar.anim(&["pump"], num::SHOTGUN_PUMP);
                ar.busy_until = ar.next_fire;
                done = true;
            }
            if !done && !ar.in_reload {
                let fire = |ar: &mut Arsenal, world: &mut SandboxWorld, cues: &mut Vec<Cue>, punch: &mut super::movement::Punch, rng: &mut super::audio::Rng, both: bool| {
                    cues.push(Cue::named(if both { "Weapon_Shotgun.Double" } else { "Weapon_Shotgun.Single" }));
                    ar.next_fire = now + if both { ar.anim(&["altfire"], num::SHOTGUN_DOUBLE) } else { ar.anim(&["fire01"], num::SHOTGUN_FIRE) };
                    ar.clips[w] -= if both { 2 } else { 1 };
                    ar.kick = if both { 1.8 } else { 1.2 };
                    // (7 pellets a barrel, `sk_plr_num_shotgun_pellets`; 12 for both. Only the first few are heard.)
                    for i in 0..if both { 12 } else { 7 } {
                        let dir = spread(rng, forward, num::CONE_10);
                        shoot(world, cues, &mut ar.fx, dir, num::BULLET_REACH, num::PELLET_IMPULSE, i < 2);
                    }
                    ar.fx.push(Fx { kind: FxKind::Flash, at: Vec3::ZERO, age: 0.0, life: 0.07 });
                    if both {
                        punch.kick(Vec3::new(rng.range(-5.0, 5.0), 0.0, 0.0));
                    } else {
                        punch.kick(Vec3::new(rng.range(-2.0, -1.0), rng.range(-2.0, 2.0), 0.0));
                    }
                    if ar.clips[w] > 0 {
                        ar.need_pump = true;
                    }
                };
                if (ar.delayed[1] || input.attack2) && now >= ar.next_fire {
                    ar.delayed[1] = false;
                    if ar.clips[w] <= 1 {
                        if ar.clips[w] == 1 {
                            fire(ar, world, &mut sb.cues, &mut sb.punch, rng, false);
                        } else if ar.reserve[w] == 0 {
                            dry(ar, &mut sb.cues);
                        } else {
                            start_reload(ar);
                        }
                    } else {
                        fire(ar, world, &mut sb.cues, &mut sb.punch, rng, true);
                    }
                } else if (ar.delayed[0] || input.attack) && now >= ar.next_fire {
                    ar.delayed[0] = false;
                    if ar.clips[w] == 0 {
                        if ar.reserve[w] == 0 {
                            dry(ar, &mut sb.cues);
                        } else {
                            start_reload(ar);
                        }
                    } else {
                        fire(ar, world, &mut sb.cues, &mut sb.punch, rng, false);
                    }
                }
                if input.reload && !ar.in_reload {
                    start_reload(ar);
                } else if ar.clips[w] == 0 && now > ar.next_fire && !ar.in_reload {
                    start_reload(ar);
                }
            }
        }
        Kind::GravityGun => {
            match sb.held {
                Some(id) => {
                    // Held in front of him: `24 + 2 radius` units off.
                    let Some((com, _)) = world.motion(id) else { return };
                    let radius = world.prop(id).map_or(0.3, |p| p.radius);
                    let target = eye + forward * (24.0 * UNIT_M + radius * 1.6 + 0.35);
                    let to = target - com;
                    world.set_velocity(id, (to * 16.0).clamp_length_max(28.0), None);
                    ar.astray = if to.length() > 1.6 { ar.astray + dt } else { 0.0 };
                    sb.loop_sound = Some(("Weapon_PhysCannon.HoldSound", 1.0));
                    ar.idle(&["hold_idle"]);
                    let launch = attack_pressed && now >= ar.next_fire;
                    if launch || input.attack2_pressed || ar.astray > 0.6 {
                        world.set_held(id, false);
                        sb.held = None;
                        ar.astray = 0.0;
                        ar.next_fire = now + 0.5;
                        ar.idle(&["idle"]);
                        if launch {
                            let mass = world.prop(id).map_or(1.0, |p| p.mass);
                            let speed = num::CANNON_LAUNCH * UNIT_M * (100.0 / mass).min(1.0);
                            let spin = Vec3::new(rng.range(-3.0, 3.0), rng.range(-3.0, 3.0), rng.range(-3.0, 3.0));
                            world.set_velocity(id, forward * speed, Some(spin));
                            // `PrimaryFireEffect`: the view jumps 6 degrees, the screen flashes white.
                            ar.kick = 1.5;
                            sb.punch.kick(Vec3::new(-6.0, rng.pick(5) as f32 - 2.0, 0.0));
                            ar.fx.push(Fx { kind: FxKind::White, at: Vec3::ZERO, age: 0.0, life: 0.1 });
                            sb.cues.push(Cue::named("Weapon_PhysCannon.Launch"));
                            ar.anim(&["altfire", "fire"], 0.4);
                        } else {
                            sb.cues.push(Cue::named("Weapon_PhysCannon.Drop"));
                        }
                    }
                }
                None => {
                    ar.idle(&["idle"]);
                    let target = cannon_target(world, eye, forward);
                    // `CheckForTarget`: the claws open on something it could lift, and close half a second after.
                    let liftable = target.and_then(|id| world.prop(id)).is_some_and(|p| p.mass <= num::CANNON_MAX_MASS && !p.frozen);
                    if liftable {
                        if !ar.claws {
                            ar.claws = true;
                            sb.cues.push(Cue::named("Weapon_PhysCannon.OpenClaws"));
                        }
                        ar.claws_close_at = now + 0.5;
                    } else if ar.claws && now >= ar.claws_close_at {
                        ar.claws = false;
                        sb.cues.push(Cue::named("Weapon_PhysCannon.CloseClaws"));
                    }
                    if input.attack2_pressed {
                        match target.and_then(|id| world.prop(id).map(|p| (id, p.mass, p.frozen))) {
                            Some((id, mass, frozen)) if mass <= num::CANNON_MAX_MASS && !frozen => {
                                world.set_held(id, true);
                                sb.held = Some(id);
                                ar.astray = 0.0;
                                sb.cues.push(Cue::named("Weapon_PhysCannon.Pickup"));
                            }
                            Some(_) => sb.cues.push(Cue::named("Weapon_PhysCannon.TooHeavy")),
                            None => {
                                sb.cues.push(Cue::named("Weapon_PhysCannon.DryFire"));
                                ar.anim(&["fire"], 0.4);
                            }
                        }
                    } else if attack_pressed && now >= ar.next_fire {
                        ar.next_fire = now + 0.5;
                        match target.and_then(|id| world.prop(id).map(|p| (id, p.mass))) {
                            Some((id, mass)) => {
                                // A punt: looking down it still goes up and away (`forward.z *= -0.65`).
                                let mut dir = forward;
                                if dir.z < 0.0 {
                                    dir.z *= -0.65;
                                }
                                let dir = dir.normalize();
                                let Some((com, _)) = world.motion(id) else { return };
                                let dv = (num::CANNON_PUNT * 0.0254 / mass).min(num::CANNON_LAUNCH * UNIT_M);
                                world.impulse(id, dir * (dv * mass), com + Vec3::new(rng.range(-0.03, 0.03), rng.range(-0.03, 0.03), 0.0));
                                ar.kick = 1.5;
                                sb.punch.kick(Vec3::new(-6.0, rng.pick(5) as f32 - 2.0, 0.0));
                                ar.fx.push(Fx { kind: FxKind::White, at: Vec3::ZERO, age: 0.0, life: 0.1 });
                                sb.cues.push(Cue::named("Weapon_PhysCannon.Launch"));
                                ar.anim(&["altfire", "fire"], 0.4);
                            }
                            None => {
                                // `DryFire`.
                                sb.cues.push(Cue::named("Weapon_PhysCannon.DryFire"));
                                ar.anim(&["fire"], 0.4);
                            }
                        }
                    }
                }
            }
        }
        Kind::PhysGun => {
            if input.reload_pressed {
                // R lets go what he looks at; nothing under the cross, everything.
                let looked = world.ray(eye, forward, num::PHYS_REACH, None).and_then(|h| if let Met::Prop(id) = h.met { Some(id) } else { None });
                let ids: Vec<u32> = match looked {
                    Some(id) => vec![id],
                    None => world.props().filter(|p| p.frozen).map(|p| p.id).collect(),
                };
                let n = ids.iter().filter(|id| world.prop(**id).is_some_and(|p| p.frozen)).count();
                for id in ids {
                    world.set_frozen(id, false);
                }
                if n > 0 {
                    sb.cues.push(Cue::new("npc/roller/mine/rmine_chirp_answer1.wav", 0.5, 1.0));
                }
            }
            match sb.held {
                Some(id) => {
                    let Some((com, _)) = world.motion(id) else { return };
                    ar.hold_dist = (ar.hold_dist + input.wheel * num::PHYS_WHEEL).clamp(0.9, num::PHYS_REACH);
                    let target = eye + forward * ar.hold_dist;
                    // E: the mouse turns the prop, not the view.
                    sb.rotating = input.use_held;
                    let spin = if input.use_held {
                        let right = forward.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
                        (Vec3::Z * sb.spin.x + right * sb.spin.y) * (0.006 / dt.max(0.002))
                    } else {
                        Vec3::ZERO
                    };
                    world.set_velocity(id, ((target - com) * 18.0).clamp_length_max(60.0), Some(spin));
                    sb.loop_sound = Some(("Weapon_MegaPhysCannon.HoldSound", 0.55));
                    ar.idle(&["hold_idle"]);
                    if input.attack2_pressed {
                        // Frozen where it is.
                        world.set_velocity(id, Vec3::ZERO, Some(Vec3::ZERO));
                        world.set_held(id, false);
                        world.set_frozen(id, true);
                        sb.held = None;
                        sb.cues.push(Cue::named("Weapon_Physgun.Off").louder(1.0));
                        ar.idle(&["idle"]);
                    } else if !input.attack {
                        world.set_held(id, false);
                        sb.held = None;
                        sb.cues.push(Cue::named("Weapon_Physgun.Off"));
                        ar.idle(&["idle"]);
                    }
                }
                None => {
                    ar.idle(&["idle"]);
                    if attack_pressed && now >= ar.next_fire {
                        if let Some(hit) = world.ray(eye, forward, num::PHYS_REACH, None) {
                            if let Met::Prop(id) = hit.met {
                                world.set_frozen(id, false);
                                world.set_held(id, true);
                                sb.held = Some(id);
                                let com = world.motion(id).map_or(hit.point, |m| m.0);
                                ar.hold_dist = (com - eye).dot(forward).clamp(0.9, num::PHYS_REACH);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The view model's triangles on the screen, with their bounds, to tell where the gun covers its beam.
struct Outline {
    tris: Vec<([egui::Pos2; 3], egui::Rect)>,
    bounds: egui::Rect,
}

impl Outline {
    fn covers(&self, q: egui::Pos2) -> bool {
        if !self.bounds.contains(q) {
            return false;
        }
        let side = |a: egui::Pos2, b: egui::Pos2| (b.x - a.x) * (q.y - a.y) - (b.y - a.y) * (q.x - a.x);
        self.tris.iter().any(|([a, b, c], r)| {
            if !r.contains(q) {
                return false;
            }
            let (d0, d1, d2) = (side(*a, *b), side(*b, *c), side(*c, *a));
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
        })
    }
}

/// Projects the gun's triangles (render frame, camera-relative) to egui's points. Its own projection rather than
/// `world_to_viewport`, which drops what is nearer than the camera's near plane, as the hands can be.
fn gun_outline(tris: &[[Vec3; 3]], camera: Option<(&Camera, &GlobalTransform)>) -> Outline {
    let mut out = Outline { tris: Vec::new(), bounds: egui::Rect::NOTHING };
    let Some((cam, gt)) = camera else { return out };
    let Some(rect) = cam.logical_viewport_rect() else { return out };
    let view_from_world = Mat4::from(gt.affine().inverse());
    let clip_from_view = cam.clip_from_view();
    let project = |p: Vec3| -> Option<egui::Pos2> {
        let v = view_from_world.transform_point3(p);
        if v.z > -1e-4 {
            return None;
        }
        let ndc = clip_from_view.project_point3(v);
        let at = (Vec2::new(ndc.x, -ndc.y) + Vec2::ONE) * 0.5 * rect.size() + rect.min;
        Some(egui::pos2(at.x, at.y))
    };
    for t in tris {
        let (Some(a), Some(b), Some(c)) = (project(t[0]), project(t[1]), project(t[2])) else { continue };
        let r = egui::Rect::from_points(&[a, b, c]);
        out.bounds = out.bounds.union(r);
        out.tris.push(([a, b, c], r));
    }
    out
}

/// Where along `curve` (0 the gun, 1 what it holds) the beam comes out from behind the gun: just past the last point that
/// `covered` says the gun hides, walking back from the far end, found to a fine step. 1 if all of it is hidden.
fn beam_start(curve: &impl Fn(f32) -> egui::Pos2, covered: impl Fn(egui::Pos2) -> bool) -> f32 {
    const STEPS: usize = 64;
    for i in (0..=STEPS).rev() {
        let t = i as f32 / STEPS as f32;
        if covered(curve(t)) {
            let (mut lo, mut hi) = (t, (t + 1.0 / STEPS as f32).min(1.0));
            for _ in 0..8 {
                let m = 0.5 * (lo + hi);
                if covered(curve(m)) {
                    lo = m;
                } else {
                    hi = m;
                }
            }
            return hi;
        }
    }
    0.0
}

/// One mesh of the stand-in weapon in his hands.
#[derive(Component)]
pub struct ViewPart;

/// The weapon in his hands. With the game it is Half-Life 2's view model: made in the scene once the worker has read it,
/// posed each frame by its sequence and by Source's own sway (`viewmodel.rs`). Otherwise the world model or a plain
/// stand-in, placed before the eyes with a sway, a bob and a kick of the port's own.
#[allow(clippy::too_many_arguments)]
pub fn place_weapon(
    mut commands: Commands,
    time: Res<Time<Real>>,
    sb: Res<Sandbox>,
    cam: Res<FlyCamera>,
    mut ar: ResMut<Arsenal>,
    mut assets: ResMut<GmodAssets>,
    light: Option<Res<PropLight>>,
    bodies: Query<&SystemBody>,
    (mut meshes, mut images, mut materials, mut view_materials, mut buffers): (ResMut<Assets<Mesh>>, ResMut<Assets<Image>>, ResMut<Assets<PropMaterial>>, ResMut<Assets<ViewMaterial>>, ResMut<Assets<ShaderBuffer>>),
    mut parts: Query<(&mut Transform, &mut Visibility), (With<ViewPart>, Without<ViewModelPart>)>,
    mut view_parts: Query<&mut Visibility, (With<ViewModelPart>, Without<ViewPart>)>,
) {
    let Some(light) = light else { return };
    let ar = &mut *ar;
    let dt = time.delta_secs().min(0.1);
    // The view models the worker read go into the scene one a frame (their textures are the cost), from the start: by
    // the time he is set down they are all there.
    if assets.looked {
        if let Some(i) = (0..WEAPONS.len()).find(|i| !ar.view_made[*i]) {
            ar.view_made[i] = true;
            if let Some(data) = assets.views.get_mut(i).and_then(|v| v.take()) {
                let white = assets.white(&mut images);
                ar.view[i] = Some(ViewModel::spawn(&mut commands, data, &light, &mut meshes, &mut images, &mut view_materials, &mut buffers, white));
                if i == ar.current {
                    // (It is in hand already: brought up afresh, now with its draw.)
                    ar.drawn = false;
                }
            }
        }
    }
    // (His hands are full of a prop: the weapon is away.)
    let show = sb.walking() && sb.placed && !(sb.by_hand && sb.held.is_some());
    let w = ar.current;
    let has_view = ar.view[w].is_some();
    ar.muzzle = None;
    ar.gun_tris.clear();
    let world = sb.world.as_ref();
    let body = world.and_then(|w| bodies.iter().find(|b| b.obj == w.frame.obj));
    for (i, v) in ar.view.iter().enumerate() {
        let Some(v) = v else { continue };
        let want = if show && i == w { Visibility::Visible } else { Visibility::Hidden };
        for e in &v.parts {
            if let Ok(mut vis) = view_parts.get_mut(*e) {
                if *vis != want {
                    *vis = want;
                }
            }
        }
    }
    if let (true, Some(world), Some(b)) = (has_view, world, body) {
        // Half-Life 2's view model: at his eyes, along the view without its punch, with the bob and the lag.
        let speed = sb.player.velocity.truncate().length();
        if !show {
            ar.sway.settle(sb.yaw, sb.pitch);
        }
        let (origin, axes) = ar.sway.place(sb.yaw, sb.pitch, speed, dt);
        let to_render = (b.orient * world.frame.rotation()).as_quat();
        let place = viewmodel::placement(origin, axes, to_render);
        // The gravity gun's prongs: open on what it could lift and round what it holds.
        let open = ar.claws || (matches!(WEAPONS[w].kind, Kind::GravityGun) && sb.held.is_some() && !sb.by_hand);
        ar.claws_pos = if open { (ar.claws_pos + dt / 0.2).min(1.0) } else { (ar.claws_pos - dt / 0.5).max(0.0) };
        let prongs = ar.claws_pos * ar.claws_pos * (3.0 - 2.0 * ar.claws_pos);
        if let Some(v) = ar.view[w].as_mut() {
            v.open = prongs;
            v.advance(dt);
            if show {
                // The muzzle as the screen has it: the view model is drawn with its own narrower field of view.
                let cam_rot = cam.rot.as_quat();
                let on_screen = |m: Vec3| {
                    let c = cam_rot.inverse() * m;
                    cam_rot * Vec3::new(c.x * viewmodel::FOV_RATIO, c.y * viewmodel::FOV_RATIO, c.z)
                };
                ar.muzzle = v.pose(place, &mut buffers).map(on_screen);
                if sb.held.is_some() && !sb.by_hand {
                    v.outline(on_screen, &mut ar.gun_tris);
                }
            }
        }
    }
    // The stand-in: made the first time it is needed.
    if show && !has_view && ar.view_made[w] && !ar.made[w] && assets.looked {
        ar.made[w] = true;
        let def = &WEAPONS[w];
        let mut gpu = Gpu { meshes: &mut meshes, images: &mut images, materials: &mut materials, light: &light };
        let path = if assets.install.is_some() { def.model } else { def.stand_in };
        let found = assets.prop(&mut gpu, path).map(|a| a.parts.clone());
        let model = match found {
            Some(p) => Some(p),
            None => assets.prop(&mut gpu, def.stand_in).map(|a| a.parts.clone()),
        };
        ar.parts[w] = model
            .unwrap_or_default()
            .into_iter()
            .map(|(mesh, material)| {
                commands
                    .spawn((Name::new("Sandbox weapon"), ViewPart, Mesh3d(mesh), MeshMaterial3d(material), Transform::default(), Visibility::Hidden, NoFrustumCulling, RenderLayers::layer(crate::ship::warp::SHIP_LAYER)))
                    .id()
            })
            .collect();
    }
    // It lags behind the view's turn and bobs with his steps.
    let turn = Vec2::new(angle_diff(sb.yaw, ar.last_yaw), sb.pitch - ar.last_pitch);
    ar.last_yaw = sb.yaw;
    ar.last_pitch = sb.pitch;
    ar.lag = (ar.lag + turn * 0.6).clamp_length_max(0.06) * (-dt * 9.0).exp();
    let speed = sb.player.velocity.truncate().length().min(320.0);
    if sb.player.on_ground() {
        ar.bob += dt * speed / 320.0 * 9.0;
    }
    let bob = Vec2::new((ar.bob * 0.5).sin(), (ar.bob).sin().abs()) * (speed / 320.0) * 0.012;
    let def = &WEAPONS[w];
    let reloading = ar.reload_until.is_some() || ar.in_reload;
    // Coming up from below over the deploy; dipping while it's loaded.
    let rise = 1.0 - (ar.deployed / (num::DEPLOY * 0.8)).clamp(0.0, 1.0);
    let dip = rise * rise * 0.3 + if reloading { 0.07 } else { 0.0 };
    let kick = ar.kick.min(2.0);
    let mut offset = def.hold + Vec3::new(ar.lag.x * 0.5 + bob.x, -dip - bob.y + ar.lag.y * 0.4, -kick * 0.035);
    let mut tilt = Quat::from_rotation_x(kick * 0.05 - dip * 1.2) * Quat::from_rotation_y(-ar.lag.x * 0.8 + 0.035);
    if def.kind == Kind::Crowbar {
        // Held up at rest; the swing comes across and down.
        let s = ar.swing;
        let arc = (s * std::f32::consts::PI).sin();
        tilt = Quat::from_rotation_y(0.5 + arc * 0.9) * Quat::from_rotation_x(0.9 - arc * 1.3) * tilt;
        offset += Vec3::new(-arc * 0.16, arc * 0.05, arc * 0.12);
    }
    // The model's axes (x right, y forward or back, z up) to the camera's (x right, y up, -z forward).
    let to_view = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2) * if def.backwards { Quat::from_rotation_z(std::f32::consts::PI) } else { Quat::IDENTITY };
    let cam_rot = cam.rot.as_quat();
    let tf = Transform { translation: cam_rot * Vec3::new(offset.x, offset.y, -offset.z), rotation: cam_rot * tilt * to_view, scale: Vec3::splat(def.scale) };
    for (i, list) in ar.parts.iter().enumerate() {
        for e in list {
            let Ok((mut transform, mut vis)) = parts.get_mut(*e) else { continue };
            let want = if show && i == w && !has_view { Visibility::Visible } else { Visibility::Hidden };
            if *vis != want {
                *vis = want;
            }
            if i == w {
                *transform = tf;
            }
        }
    }
    if show && !has_view {
        // (The stand-in's muzzle: ahead of where it is held.)
        ar.muzzle = Some(cam_rot * Vec3::new(def.hold.x * 0.9, def.hold.y * 0.8, -(def.hold.z + 0.25)));
    }
    // The HUD draws in screen space: where the held prop and this frame's effects are in the render frame.
    ar.beam_to = None;
    ar.fx_render.clear();
    let (Some(world), Some(b)) = (world, body) else { return };
    // (A gun's hold shows its beam; his hands' doesn't.)
    if let Some(prop) = sb.held.filter(|_| !sb.by_hand).and_then(|id| world.prop(id)) {
        ar.beam_to = Some(super::props::render_point(b, &world.frame, &cam, prop.prev.pos.lerp(prop.now.pos, sb.tick_fraction())));
    }
    for f in &ar.fx {
        ar.fx_render.push((*f, super::props::render_point(b, &world.frame, &cam, f.at)));
    }
}

fn angle_diff(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI { d - std::f32::consts::TAU } else { d }
}

/// Half-Life 2's HUD colours (`ClientScheme.res`): the yellow of its numbers, the dark of its panels.
mod hud {
    use bevy_egui::egui::Color32;
    pub const YELLOW: Color32 = Color32::from_rgb(255, 220, 0);
    pub const LABEL: Color32 = Color32::from_rgba_premultiplied(200, 172, 0, 200);
    pub const PANEL: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 76);
    pub const RED: Color32 = Color32::from_rgb(255, 48, 0);
}

/// The HUD while he walks: the cross, health, ammunition, the weapon selection, and this frame's flashes and dust.
pub fn draw_hud(mut contexts: EguiContexts, sb: Res<Sandbox>, ar: Res<Arsenal>, camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>, control: Res<crate::mods::ModControl>) -> Result {
    if !sb.walking() || control.main_menu {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let screen = ctx.content_rect();
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("sandbox_hud")));
    let scale = (screen.height() / 900.0).clamp(0.8, 1.6);
    let c = screen.center();
    let project = |at: Vec3| -> Option<(egui::Pos2, f32)> {
        let (cam, gt) = camera.single().ok()?;
        let dist = at.length();
        let v = cam.world_to_viewport(gt, at).ok()?;
        Some((egui::pos2(v.x, v.y), dist))
    };
    if !sb.placed {
        p.text(c + egui::vec2(0.0, 40.0), egui::Align2::CENTER_CENTER, "Waiting for the ground...", egui::FontId::proportional(18.0 * scale), hud::YELLOW);
        return Ok(());
    }
    // Where the muzzle is on the screen (the weapon's own, else a little right and below the middle).
    let muzzle = ar.muzzle.and_then(project).map_or(c + egui::vec2(0.2 * screen.height(), 0.2 * screen.height()), |m| m.0);
    // The world's effects, under everything else of the HUD.
    for (f, at) in &ar.fx_render {
        let t = f.age / f.life;
        match f.kind {
            FxKind::Dust => {
                if let Some((pos, dist)) = project(*at + Vec3::ZERO) {
                    let r = (0.05 + 0.3 * t) / dist.max(0.3) * screen.height() * 0.9;
                    p.circle_filled(pos - egui::vec2(0.0, r * 0.5 * t), r.min(160.0), egui::Color32::from_rgba_unmultiplied(150, 130, 105, ((1.0 - t) * 110.0) as u8));
                }
            }
            FxKind::Spark => {
                if let Some((pos, dist)) = project(*at) {
                    let r = (0.035 * (1.0 - t * 0.5)) / dist.max(0.3) * screen.height() * 0.9;
                    p.circle_filled(pos, r.clamp(1.5, 60.0), egui::Color32::from_rgba_unmultiplied(255, 225, 150, ((1.0 - t) * 230.0) as u8));
                }
            }
            FxKind::Flash => {
                let r = 46.0 * scale * (1.0 - t * 0.6);
                p.circle_filled(muzzle, r, egui::Color32::from_rgba_unmultiplied(255, 190, 70, ((1.0 - t) * 90.0) as u8));
                p.circle_filled(muzzle, r * 0.5, egui::Color32::from_rgba_unmultiplied(255, 245, 200, ((1.0 - t) * 200.0) as u8));
            }
            FxKind::White => {
                p.rect_filled(screen, 0.0, egui::Color32::from_rgba_unmultiplied(245, 245, 255, ((1.0 - t) * 32.0) as u8));
            }
        }
    }
    // The physics gun's beam (and the gravity gun's hold): from the muzzle to what is held.
    if let Some((to, _)) = ar.beam_to.and_then(project) {
        let from = muzzle;
        let (wide, core) = if WEAPONS[ar.current].kind == Kind::PhysGun { (egui::Color32::from_rgba_unmultiplied(60, 160, 255, 70), egui::Color32::from_rgba_unmultiplied(190, 235, 255, 230)) } else { (egui::Color32::from_rgba_unmultiplied(255, 150, 40, 40), egui::Color32::from_rgba_unmultiplied(255, 215, 150, 120)) };
        // A sagging curve, as the physics gun's beam bends.
        let mid = from.lerp(to, 0.5) + egui::vec2(0.0, 18.0 * scale);
        let curve = |t: f32| from.lerp(mid, t).lerp(mid.lerp(to, t), t);
        // The gun is in front of its beam: it starts where the curve last leaves the gun's outline.
        let outline = gun_outline(&ar.gun_tris, camera.single().ok());
        let start = beam_start(&curve, |q| outline.covers(q));
        if start < 1.0 {
            let points: Vec<egui::Pos2> = (0..=24).map(|i| curve(start + (1.0 - start) * i as f32 / 24.0)).collect();
            p.add(egui::Shape::line(points.clone(), egui::Stroke::new(7.0 * scale, wide)));
            p.add(egui::Shape::line(points, egui::Stroke::new(2.0 * scale, core)));
        }
    }
    if sb.menu {
        return Ok(());
    }
    // The cross.
    let (gap, len) = (4.0 * scale, 6.0 * scale);
    let ink = egui::Stroke::new(1.6 * scale, egui::Color32::from_rgba_unmultiplied(255, 245, 190, 210));
    for d in [egui::vec2(1.0, 0.0), egui::vec2(-1.0, 0.0), egui::vec2(0.0, 1.0), egui::vec2(0.0, -1.0)] {
        p.line_segment([c + d * gap, c + d * (gap + len)], ink);
    }
    p.circle_filled(c, 1.0 * scale, ink.color);
    // Health, bottom left; ammunition, bottom right.
    let big = egui::FontId::proportional(40.0 * scale);
    let small = egui::FontId::proportional(13.0 * scale);
    let panel = |min: egui::Pos2, w: f32| egui::Rect::from_min_size(min, egui::vec2(w * scale, 54.0 * scale));
    let r = panel(egui::pos2(screen.left() + 24.0 * scale, screen.bottom() - 78.0 * scale), 150.0);
    p.rect_filled(r, 8.0 * scale, hud::PANEL);
    let ink = if ar.health <= 25 { hud::RED } else { hud::YELLOW };
    p.text(r.left_bottom() + egui::vec2(12.0 * scale, -9.0 * scale), egui::Align2::LEFT_BOTTOM, "HEALTH", small.clone(), if ar.health <= 25 { hud::RED } else { hud::LABEL });
    p.text(r.right_center() - egui::vec2(12.0 * scale, 0.0), egui::Align2::RIGHT_CENTER, ar.health.to_string(), big.clone(), ink);
    let def = &WEAPONS[ar.current];
    if def.clip > 0 {
        let r = panel(egui::pos2(screen.right() - (24.0 + 210.0) * scale, screen.bottom() - 78.0 * scale), 210.0);
        p.rect_filled(r, 8.0 * scale, hud::PANEL);
        let (clip, reserve) = (ar.clips[ar.current], ar.reserve[ar.current]);
        let ink = if clip == 0 { hud::RED } else { hud::YELLOW };
        p.text(r.left_bottom() + egui::vec2(12.0 * scale, -9.0 * scale), egui::Align2::LEFT_BOTTOM, "AMMO", small.clone(), hud::LABEL);
        p.text(r.left_center() + egui::vec2(118.0 * scale, 0.0), egui::Align2::RIGHT_CENTER, clip.to_string(), big.clone(), ink);
        p.text(r.right_center() - egui::vec2(14.0 * scale, -6.0 * scale), egui::Align2::RIGHT_CENTER, reserve.to_string(), egui::FontId::proportional(24.0 * scale), hud::YELLOW);
    }
    // The selection: every weapon by its slot across the top, the highlighted one named.
    if let Some((sel, _)) = ar.selecting {
        let (bw, bh, gap) = (118.0 * scale, 46.0 * scale, 6.0 * scale);
        let total = WEAPONS.len() as f32 * (bw + gap) - gap;
        let mut x = c.x - total * 0.5;
        for (i, w) in WEAPONS.iter().enumerate() {
            let r = egui::Rect::from_min_size(egui::pos2(x, screen.top() + 28.0 * scale), egui::vec2(bw, bh));
            let on = i == sel;
            p.rect_filled(r, 6.0 * scale, if on { egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150) } else { hud::PANEL });
            if on {
                p.rect_stroke(r, 6.0 * scale, egui::Stroke::new(1.5 * scale, hud::YELLOW), egui::StrokeKind::Inside);
            }
            p.text(r.left_top() + egui::vec2(6.0 * scale, 4.0 * scale), egui::Align2::LEFT_TOP, w.slot.to_string(), small.clone(), hud::LABEL);
            p.text(r.center() + egui::vec2(0.0, 5.0 * scale), egui::Align2::CENTER_CENTER, w.name, egui::FontId::proportional(14.5 * scale), if on { hud::YELLOW } else { hud::LABEL });
            x += bw + gap;
        }
    }
    // The keys, for the first half minute.
    if ar.time < ar.hint_until {
        let fade = ((ar.hint_until - ar.time) / 3.0).clamp(0.0, 1.0);
        let text = "WASD move   ·   Space jump   ·   Ctrl duck   ·   Shift run   ·   hold Q: spawn menu   ·   1-4, wheel: weapons   ·   Z undo   ·   V free flight";
        p.text(egui::pos2(c.x, screen.bottom() - 18.0 * scale), egui::Align2::CENTER_BOTTOM, text, egui::FontId::proportional(13.5 * scale), hud::YELLOW.gamma_multiply(0.75 * fade));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shots_spread_as_fire_bullets_does() {
        let mut rng = super::super::audio::Rng(3);
        let forward = Vec3::new(0.6, 0.8, 0.0);
        let (mut inner, n) = (0, 4000);
        for _ in 0..n {
            let d = spread(&mut rng, forward, num::CONE_5);
            let off = (d - forward * d.dot(forward)).length() / d.dot(forward);
            assert!(off <= num::CONE_5 * 1.001, "{off}");
            if off < num::CONE_5 * 0.5 {
                inner += 1;
            }
        }
        // Two summed numbers each way: half the shots (0.496 of them, worked out from the two triangles) land in the
        // inner half of the cone; evenly over the disc it would be a quarter.
        let share = inner as f32 / n as f32;
        assert!((0.46..0.53).contains(&share), "{inner}");
    }

    #[test]
    fn the_beam_starts_where_it_leaves_the_gun() {
        // A gun covering x 0..100 (two triangles); the beam from x 50 to x 300 along y 50.
        let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
            let p = [egui::pos2(a.0, a.1), egui::pos2(b.0, b.1), egui::pos2(c.0, c.1)];
            (p, egui::Rect::from_points(&p))
        };
        let tris = vec![tri((0.0, 0.0), (100.0, 0.0), (100.0, 100.0)), tri((0.0, 0.0), (100.0, 100.0), (0.0, 100.0))];
        let outline = Outline { bounds: tris.iter().fold(egui::Rect::NOTHING, |r, t| r.union(t.1)), tris };
        let curve = |t: f32| egui::pos2(50.0 + 250.0 * t, 50.0);
        let start = beam_start(&curve, |q| outline.covers(q));
        assert!((curve(start).x - 100.0).abs() < 0.5, "starts at x {}", curve(start).x);
        // Nothing in the way: all of it; all of it hidden: none.
        assert_eq!(beam_start(&curve, |_| false), 0.0);
        assert_eq!(beam_start(&curve, |_| true), 1.0);
    }

    #[test]
    fn every_weapon_names_a_view_model_and_its_sequences() {
        for w in &WEAPONS {
            assert!(w.view.starts_with("models/weapons/v_") && w.view.ends_with(".mdl"));
            assert!(w.sequences().contains(&"draw"));
            assert!(w.sequences().iter().any(|s| *s == "idle01" || *s == "idle"));
        }
    }
}
