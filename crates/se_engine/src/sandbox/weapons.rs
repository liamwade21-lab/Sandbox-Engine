//! The sandbox's weapons: a small set of Garry's Mod's default loadout (`player_sandbox.lua`), with Half-Life 2's numbers
//! (Source SDK 2013 `weapon_pistol.cpp`, `weapon_smg1.cpp`, `weapon_shotgun.cpp`, `weapon_crowbar.cpp`,
//! `weapon_physcannon.cpp`, `hl2mp_gamerules.cpp`'s bullet impulses) and its sounds (`game_sounds_weapons.txt`).
//! Bullets are rays in the sandbox's world: they push props and kick up dust. The gravity gun picks props up and punts
//! them; the physics gun holds one at any distance, turns it (E) and freezes it (right button).
//!
//! The weapon in his hands is Half-Life 2's world model (`models/weapons/w_*.mdl`), held where a view model would be and
//! moved by code (the sway, the bob, the kick): its real view models are animated skeletons the port has no reader for yet.
//! The HUD is Half-Life 2's: health and ammunition in its yellow on dark panels, the weapon selection across the top.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::camera::{FlyCamera, MainCamera};
use crate::system::SystemBody;

use super::assets::{Gpu, GmodAssets};
use super::audio::{Cue, bullet_impact};
use super::light::{PropLight, PropMaterial};
use super::movement::UNIT_M;
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
    /// The world model held in his hands, and a plain stand-in without the game.
    model: &'static str,
    stand_in: &'static str,
    /// The model's forward is its -y (the gravity gun's), not its +y.
    backwards: bool,
    /// Where the model's origin is held: right, up, forward of the eyes (m), and how big it is drawn (a view model is
    /// drawn with a narrower field of view than the world: smaller and further off than a real hand would hold it).
    hold: Vec3,
    scale: f32,
    /// Rounds in a full clip (0: it has none), and carried at the start (Garry's Mod's loadout).
    clip: u32,
    reserve: u32,
}

pub const WEAPONS: [WeaponDef; 6] = [
    WeaponDef { kind: Kind::Crowbar, name: "Crowbar", slot: 1, hint: "Left: swing", model: "models/weapons/w_crowbar.mdl", stand_in: "placeholder/bar", backwards: false, hold: Vec3::new(0.21, -0.27, 0.42), scale: 0.8, clip: 0, reserve: 0 },
    WeaponDef { kind: Kind::GravityGun, name: "Gravity Gun", slot: 1, hint: "Left: punt   Right: pick up / drop", model: "models/weapons/w_physics.mdl", stand_in: "placeholder/gun", backwards: true, hold: Vec3::new(0.22, -0.25, 0.40), scale: 0.72, clip: 0, reserve: 0 },
    WeaponDef { kind: Kind::PhysGun, name: "Physics Gun", slot: 1, hint: "Hold left: carry   Wheel: distance\nE: turn   Right: freeze   R: unfreeze", model: "models/weapons/w_physics.mdl", stand_in: "placeholder/gun", backwards: true, hold: Vec3::new(0.22, -0.25, 0.40), scale: 0.72, clip: 0, reserve: 0 },
    WeaponDef { kind: Kind::Pistol, name: "9mm Pistol", slot: 2, hint: "Left: fire   R: reload", model: "models/weapons/w_pistol.mdl", stand_in: "placeholder/gun", backwards: false, hold: Vec3::new(0.15, -0.15, 0.50), scale: 0.85, clip: 18, reserve: 256 },
    WeaponDef { kind: Kind::Smg, name: "SMG", slot: 3, hint: "Hold left: fire   R: reload", model: "models/weapons/w_smg1.mdl", stand_in: "placeholder/gun", backwards: false, hold: Vec3::new(0.16, -0.19, 0.45), scale: 0.8, clip: 45, reserve: 256 },
    WeaponDef { kind: Kind::Shotgun, name: "Shotgun", slot: 4, hint: "Left: fire   Right: both barrels\nR: reload", model: "models/weapons/w_shotgun.mdl", stand_in: "placeholder/gun", backwards: false, hold: Vec3::new(0.16, -0.19, 0.50), scale: 0.8, clip: 6, reserve: 64 },
];

/// Half-Life 2's numbers.
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
    pub const CONE_PISTOL: f32 = 0.013;
    pub const CONE_5: f32 = 0.043_62;
    pub const CONE_10: f32 = 0.087_16;
    /// A bullet's reach, units (`MAX_TRACE_LENGTH` is far more; the ground field ends at 60 m).
    pub const BULLET_REACH: f32 = 4096.0;
    pub const PISTOL_REFIRE: f32 = 0.1;
    pub const PISTOL_HELD_REFIRE: f32 = 0.5;
    pub const SMG_REFIRE: f32 = 0.075;
    pub const PISTOL_RELOAD: f32 = 1.43;
    pub const SMG_RELOAD: f32 = 1.5;
    pub const SHELL_TIME: f32 = 0.5;
    pub const SHOTGUN_REFIRE: f32 = 0.85;
    pub const DEPLOY: f32 = 0.45;
    /// `+use` on a prop: picked up in his hands if it weighs no more than this (kg, `PLAYER_MAX_LIFT_MASS`) and is within
    /// reach (units, `PLAYER_USE_RADIUS`); thrown at `player_throwforce` (kg in/s).
    pub const HAND_MAX_MASS: f32 = 35.0;
    pub const HAND_REACH: f32 = 80.0;
    pub const HAND_THROW: f32 = 1000.0;
    /// The physics gun: metres a wheel notch, and its reach.
    pub const PHYS_WHEEL: f32 = 0.35;
    pub const PHYS_REACH: f32 = 70.0;
}

/// Something seen for a moment: where a bullet struck, its line, the flash at the muzzle.
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
}

#[derive(Resource)]
pub struct Arsenal {
    pub current: usize,
    clips: [u32; 6],
    reserve: [u32; 6],
    /// The weapons' clock, seconds.
    time: f32,
    next_fire: f32,
    /// When the pistol last fired (held fire is slower than clicking).
    reload_until: Option<f32>,
    /// The shotgun is loading shells one at a time; when the next goes in.
    shell_at: Option<f32>,
    pump_at: Option<f32>,
    /// The weapon highlighted in the selection, and when it closes.
    selecting: Option<(usize, f32)>,
    /// How long the SMG has been firing (its kick grows).
    burst: f32,
    /// Seconds since this weapon came up, its kick (dies away), and the crowbar's swing (1 at the start).
    deployed: f32,
    kick: f32,
    swing: f32,
    /// The view's turn last frame, for the weapon's lag behind it.
    last_yaw: f32,
    last_pitch: f32,
    lag: Vec2,
    bob: f32,
    /// How far off the gravity or physics gun holds its prop (m), and how long it has been out of reach.
    hold_dist: f32,
    astray: f32,
    fx: Vec<Fx>,
    pub health: i32,
    /// The weapon models' meshes in the scene, by weapon.
    parts: [Vec<Entity>; 6],
    made: [bool; 6],
    /// This frame's picture of things, for the HUD: where the held prop and the effects are in the render frame.
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
            reload_until: None,
            shell_at: None,
            pump_at: None,
            selecting: None,
            burst: 0.0,
            deployed: 0.0,
            kick: 0.0,
            swing: 0.0,
            last_yaw: 0.0,
            last_pitch: 0.0,
            lag: Vec2::ZERO,
            bob: 0.0,
            hold_dist: 2.0,
            astray: 0.0,
            fx: Vec::new(),
            health: 100,
            parts: Default::default(),
            made: [false; 6],
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
        self.deployed = 0.0;
        self.next_fire = self.time + num::DEPLOY;
        self.reload_until = None;
        self.shell_at = None;
        self.pump_at = None;
        self.burst = 0.0;
        self.swing = 0.0;
    }

    fn def(&self) -> &'static WeaponDef {
        &WEAPONS[self.current]
    }
}

/// A direction within a cone round `forward` (`cone` the sine of half its angle), evenly over its disc as Source's
/// `VECTOR_CONE` spread is.
fn spread(rng: &mut super::audio::Rng, forward: Vec3, cone: f32) -> Vec3 {
    let right = forward.cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(forward);
    let (r, a) = (rng.unit().sqrt() * cone, rng.unit() * std::f32::consts::TAU);
    (forward + right * (r * a.cos()) + up * (r * a.sin())).normalize()
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
pub fn weapon_frame(time: Res<Time<Real>>, input: Res<WalkInput>, mut sb: ResMut<Sandbox>, mut ar: ResMut<Arsenal>) {
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
    // E: a light prop is picked up in his hands and carried before him (Half-Life 2's `+use`); E again puts it down, the
    // left button throws it. The weapon waits meanwhile.
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
                    ar.next_fire = ar.time + 0.4;
                    if input.attack_pressed {
                        let mass = world.prop(id).map_or(1.0, |p| p.mass);
                        world.set_velocity(id, forward * (num::HAND_THROW * 0.0254 / mass).min(12.0), None);
                    }
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
                // (`HL2Player.UseDeny`.)
                _ => sb.cues.push(Cue::new("common/wpn_denyselect.wav", 0.4, 1.0)),
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
        sb.cues.push(Cue::new("common/wpn_moveselect.wav", 0.5, 1.0));
    } else if input.slot.is_some() {
        sb.cues.push(Cue::new("common/wpn_denyselect.wav", 0.5, 1.0));
    }
    let mut attack_pressed = input.attack_pressed;
    if let Some((i, until)) = ar.selecting {
        if attack_pressed && !menu_open {
            ar.selecting = None;
            ar.take(i);
            info!("sandbox: took the {}", WEAPONS[i].name);
            sb.cues.push(Cue::new("common/wpn_select.wav", 0.5, 1.0));
            attack_pressed = false;
        } else if now > until || input.attack2_pressed {
            ar.selecting = None;
            sb.cues.push(Cue::new("common/wpn_hudoff.wav", 0.4, 1.0));
        }
    }
    if menu_open || ar.selecting.is_some() {
        return;
    }
    let def = ar.def();
    let w = ar.current;
    let rng = &mut sb.rng;

    // A shot along the view: what it meets takes its push, sounds, and throws dust or a spark.
    let shoot = |world: &mut SandboxWorld, cues: &mut Vec<Cue>, fx: &mut Vec<Fx>, rng: &mut super::audio::Rng, dir: Vec3, reach: f32, impulse: f32, loud: f32| {
        let Some(hit) = world.ray(eye, dir, reach * UNIT_M, None) else { return None };
        let surface = match hit.met {
            Met::Prop(id) => {
                world.impulse(id, dir * (impulse * 0.0254), hit.point);
                world.prop(id).map(|p| p.surface)
            }
            Met::Ground => None,
        };
        let far = 1.0 / (1.0 + (hit.dist / 9.0).powi(2));
        let cue = bullet_impact(rng, surface);
        cues.push(Cue { volume: cue.volume * far * loud, ..cue });
        fx.push(Fx { kind: if surface.is_some() { FxKind::Spark } else { FxKind::Dust }, at: hit.point + hit.normal * 0.02, age: 0.0, life: if surface.is_some() { 0.12 } else { 0.4 } });
        Some(hit)
    };

    match def.kind {
        Kind::Crowbar => {
            if input.attack && now >= ar.next_fire {
                ar.next_fire = now + num::CROWBAR_REFIRE;
                ar.swing = 1.0;
                // (`AddViewKick`: pitch 1..2 down, yaw 1..2 to the left.)
                sb.punch.kick(Vec3::new(rng.range(1.0, 2.0), rng.range(-2.0, -1.0), 0.0));
                match shoot(world, &mut sb.cues, &mut ar.fx, rng, forward, num::CROWBAR_REACH, num::CROWBAR_IMPULSE, 1.0) {
                    Some(_) => sb.cues.push(rng.wave("weapons/crowbar/crowbar_impact", &[1, 2], 0.7, 90.0, 120.0)),
                    None => sb.cues.push(Cue::new("weapons/iceaxe/iceaxe_swing1.wav", 0.55, rng.range(95.0, 100.0) / 100.0)),
                }
            }
        }
        Kind::Pistol | Kind::Smg => {
            let smg = def.kind == Kind::Smg;
            if let Some(t) = ar.reload_until {
                if now >= t {
                    let take = (def.clip - ar.clips[w]).min(ar.reserve[w]);
                    ar.clips[w] += take;
                    ar.reserve[w] -= take;
                    ar.reload_until = None;
                }
            }
            let reload = |ar: &mut Arsenal, cues: &mut Vec<Cue>| {
                if ar.reload_until.is_none() && ar.clips[w] < def.clip && ar.reserve[w] > 0 {
                    ar.reload_until = Some(now + if smg { num::SMG_RELOAD } else { num::PISTOL_RELOAD });
                    ar.next_fire = now + if smg { num::SMG_RELOAD } else { num::PISTOL_RELOAD };
                    cues.push(Cue::new(if smg { "weapons/smg1/smg1_reload.wav" } else { "weapons/pistol/pistol_reload1.wav" }, 0.7, 1.0));
                }
            };
            if input.reload_pressed {
                reload(ar, &mut sb.cues);
            }
            // The pistol fires as fast as it's clicked (0.1 s apart), every half second held; the SMG every 0.075 s.
            let wants = if smg { input.attack } else { attack_pressed || input.attack };
            let ready = now >= ar.next_fire && (smg || attack_pressed || now >= ar.next_fire + (num::PISTOL_HELD_REFIRE - num::PISTOL_REFIRE));
            if smg && !input.attack {
                ar.burst = 0.0;
            }
            if wants && ready && ar.reload_until.is_none() {
                if ar.clips[w] == 0 {
                    if ar.reserve[w] > 0 {
                        reload(ar, &mut sb.cues);
                    } else if attack_pressed {
                        sb.cues.push(Cue::new("weapons/pistol/pistol_empty.wav", 0.7, 1.0));
                        ar.next_fire = now + 0.2;
                    }
                } else {
                    ar.clips[w] -= 1;
                    ar.next_fire = now + if smg { num::SMG_REFIRE } else { num::PISTOL_REFIRE };
                    ar.kick = 1.0;
                    let dir = spread(rng, forward, if smg { num::CONE_5 } else { num::CONE_PISTOL });
                    shoot(world, &mut sb.cues, &mut ar.fx, rng, dir, num::BULLET_REACH, num::BULLET_IMPULSE, 1.0);
                    ar.fx.push(Fx { kind: FxKind::Flash, at: Vec3::ZERO, age: 0.0, life: 0.05 });
                    if smg {
                        // `DoMachineGunKick`: grows over two seconds of fire, halved.
                        ar.burst += num::SMG_REFIRE;
                        let k = ar.burst.min(2.0) / 2.0;
                        let sign = |r: &mut super::audio::Rng| if r.pick(2) == 0 { 1.0 } else { -1.0 };
                        let (sy, sz) = (sign(rng), sign(rng));
                        sb.punch.kick(Vec3::new(-(0.2 + k), (0.2 + k) / 3.0 * sy, (0.1 + k / 8.0) * sz) * 0.5);
                        sb.cues.push(Cue::new("weapons/smg1/smg1_fire1.wav", 0.55, rng.range(95.0, 105.0) / 100.0));
                    } else {
                        sb.punch.kick(Vec3::new(-rng.range(0.25, 0.5), rng.range(-0.6, 0.6), 0.0));
                        sb.cues.push(Cue::new("weapons/pistol/pistol_fire2.wav", 0.55, rng.range(98.0, 102.0) / 100.0));
                    }
                }
            }
        }
        Kind::Shotgun => {
            if let Some(t) = ar.pump_at.filter(|t| now >= *t) {
                let _ = t;
                ar.pump_at = None;
                sb.cues.push(Cue::new("weapons/shotgun/shotgun_cock.wav", 0.7, 1.0));
            }
            // Shells go in one at a time; firing breaks off the loading.
            if let Some(t) = ar.shell_at {
                let stop = (attack_pressed || input.attack2_pressed) && ar.clips[w] > 0;
                if stop || ar.clips[w] >= def.clip || ar.reserve[w] == 0 {
                    ar.shell_at = None;
                    ar.next_fire = now + 0.3;
                } else if now >= t {
                    ar.clips[w] += 1;
                    ar.reserve[w] -= 1;
                    sb.cues.push(rng.wave("weapons/shotgun/shotgun_reload", &[1, 2, 3], 0.7, 100.0, 100.0));
                    ar.shell_at = Some(now + num::SHELL_TIME);
                }
            }
            let start_loading = |ar: &mut Arsenal| {
                if ar.shell_at.is_none() && ar.clips[w] < def.clip && ar.reserve[w] > 0 {
                    ar.shell_at = Some(now + num::SHELL_TIME);
                    ar.next_fire = now + num::SHELL_TIME;
                }
            };
            if input.reload_pressed {
                start_loading(ar);
            }
            let both = input.attack2_pressed || (input.attack2 && !input.attack);
            if (input.attack || both) && now >= ar.next_fire && ar.shell_at.is_none() {
                let shells = if both && ar.clips[w] >= 2 { 2 } else { 1 };
                if ar.clips[w] == 0 {
                    if ar.reserve[w] > 0 {
                        start_loading(ar);
                    } else if attack_pressed || input.attack2_pressed {
                        sb.cues.push(Cue::new("weapons/shotgun/shotgun_empty.wav", 0.7, rng.range(95.0, 100.0) / 100.0));
                        ar.next_fire = now + 0.3;
                    }
                } else {
                    ar.clips[w] -= shells;
                    ar.next_fire = now + if shells == 2 { 1.1 } else { num::SHOTGUN_REFIRE };
                    ar.pump_at = Some(now + if shells == 2 { 0.5 } else { 0.35 });
                    ar.kick = if shells == 2 { 1.8 } else { 1.2 };
                    // (7 pellets a barrel, `sk_plr_num_shotgun_pellets`; 12 for both. Only the first few are heard.)
                    for i in 0..if shells == 2 { 12 } else { 7 } {
                        let dir = spread(rng, forward, num::CONE_10);
                        shoot(world, &mut sb.cues, &mut ar.fx, rng, dir, num::BULLET_REACH, num::PELLET_IMPULSE, if i < 2 { 1.0 } else { 0.0 });
                    }
                    ar.fx.push(Fx { kind: FxKind::Flash, at: Vec3::ZERO, age: 0.0, life: 0.07 });
                    if shells == 2 {
                        sb.punch.kick(Vec3::new(rng.range(-5.0, 5.0), 0.0, 0.0));
                        sb.cues.push(Cue::new("weapons/shotgun/shotgun_dbl_fire7.wav", 1.0, rng.range(90.0, 95.0) / 100.0));
                    } else {
                        sb.punch.kick(Vec3::new(rng.range(-2.0, -1.0), rng.range(-2.0, 2.0), 0.0));
                        sb.cues.push(Cue::new("weapons/shotgun/shotgun_fire7.wav", 0.86, rng.range(98.0, 101.0) / 100.0));
                    }
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
                    sb.loop_sound = Some(("weapons/physcannon/hold_loop.wav", 0.4));
                    let launch = attack_pressed && now >= ar.next_fire;
                    if launch || input.attack2_pressed || ar.astray > 0.6 {
                        world.set_held(id, false);
                        sb.held = None;
                        ar.astray = 0.0;
                        ar.next_fire = now + 0.5;
                        if launch {
                            let mass = world.prop(id).map_or(1.0, |p| p.mass);
                            let speed = num::CANNON_LAUNCH * UNIT_M * (100.0 / mass).min(1.0);
                            let spin = Vec3::new(rng.range(-3.0, 3.0), rng.range(-3.0, 3.0), rng.range(-3.0, 3.0));
                            world.set_velocity(id, forward * speed, Some(spin));
                            ar.kick = 1.5;
                            sb.punch.kick(Vec3::new(-rng.range(1.0, 2.0), rng.range(-1.0, 1.0), 0.0));
                            sb.cues.push(rng.wave("weapons/physcannon/superphys_launch", &[1, 2, 3, 4], 0.45, 110.0, 120.0));
                        } else {
                            sb.cues.push(Cue::new("weapons/physcannon/physcannon_drop.wav", 0.4, 1.0));
                        }
                    }
                }
                None => {
                    let target = cannon_target(world, eye, forward);
                    if input.attack2_pressed {
                        match target.and_then(|id| world.prop(id).map(|p| (id, p.mass, p.frozen))) {
                            Some((id, mass, frozen)) if mass <= num::CANNON_MAX_MASS && !frozen => {
                                world.set_held(id, true);
                                sb.held = Some(id);
                                ar.astray = 0.0;
                                sb.cues.push(Cue::new("weapons/physcannon/physcannon_pickup.wav", 0.4, 1.0));
                            }
                            Some(_) => sb.cues.push(Cue::new("weapons/physcannon/physcannon_tooheavy.wav", 0.5, 1.0)),
                            None => sb.cues.push(Cue::new("weapons/physcannon/physcannon_dryfire.wav", 0.4, 1.0)),
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
                                sb.punch.kick(Vec3::new(-rng.range(1.0, 2.0), rng.range(-1.0, 1.0), 0.0));
                                sb.cues.push(rng.wave("weapons/physcannon/superphys_launch", &[1, 2, 3, 4], 0.45, 110.0, 120.0));
                            }
                            None => sb.cues.push(Cue::new("weapons/physcannon/physcannon_dryfire.wav", 0.4, 1.0)),
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
                    sb.loop_sound = Some(("weapons/physcannon/superphys_hold_loop.wav", 0.22));
                    if input.attack2_pressed {
                        // Frozen where it is.
                        world.set_velocity(id, Vec3::ZERO, Some(Vec3::ZERO));
                        world.set_held(id, false);
                        world.set_frozen(id, true);
                        sb.held = None;
                        sb.cues.push(Cue::new("weapons/physgun_off.wav", 0.5, 1.15));
                    } else if !input.attack {
                        world.set_held(id, false);
                        sb.held = None;
                        sb.cues.push(Cue::new("weapons/physgun_off.wav", 0.4, 1.0));
                    }
                }
                None => {
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

/// One mesh of the weapon in his hands.
#[derive(Component)]
pub struct ViewPart;

/// The weapon in his hands: made the first time it's taken, shown only while he walks, and placed before the eyes with its
/// sway, bob and kick.
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
    (mut meshes, mut images, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<Image>>, ResMut<Assets<PropMaterial>>),
    mut parts: Query<(&mut Transform, &mut Visibility), With<ViewPart>>,
) {
    let Some(light) = light else { return };
    let ar = &mut *ar;
    let show = sb.walking() && sb.placed;
    let w = ar.current;
    if show && !ar.made[w] && assets.looked {
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
    let dt = time.delta_secs().min(0.1);
    // The weapon lags behind the view's turn (`CalcViewModelLag`) and bobs with his steps.
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
    let reloading = ar.reload_until.is_some() || ar.shell_at.is_some();
    // Coming up from below over the deploy; dipping while it's loaded.
    let rise = 1.0 - (ar.deployed / (num::DEPLOY * 0.8)).clamp(0.0, 1.0);
    let dip = rise * rise * 0.3 + if reloading { 0.07 } else { 0.0 } + if sb.by_hand && sb.held.is_some() { 0.45 } else { 0.0 };
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
            let want = if show && i == w { Visibility::Visible } else { Visibility::Hidden };
            if *vis != want {
                *vis = want;
            }
            if i == w {
                *transform = tf;
            }
        }
    }
    // The HUD draws in screen space: where the held prop and this frame's effects are in the render frame.
    ar.beam_to = None;
    ar.fx_render.clear();
    let Some(world) = sb.world.as_ref() else { return };
    let Some(b) = bodies.iter().find(|b| b.obj == world.frame.obj) else { return };
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
pub fn draw_hud(mut contexts: EguiContexts, sb: Res<Sandbox>, ar: Res<Arsenal>, camera: Query<(&Camera, &GlobalTransform), With<MainCamera>>) -> Result {
    if !sb.walking() {
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
                // At the muzzle: ahead of where the weapon is held.
                let hold = WEAPONS[ar.current].hold;
                let at = egui::pos2(c.x + hold.x * screen.height() * 1.4, c.y - hold.y * screen.height() * 1.36);
                let r = 46.0 * scale * (1.0 - t * 0.6);
                p.circle_filled(at, r, egui::Color32::from_rgba_unmultiplied(255, 190, 70, ((1.0 - t) * 90.0) as u8));
                p.circle_filled(at, r * 0.5, egui::Color32::from_rgba_unmultiplied(255, 245, 200, ((1.0 - t) * 200.0) as u8));
            }
        }
    }
    // The physics gun's beam (and the gravity gun's hold): from the muzzle to what is held.
    if let Some((to, _)) = ar.beam_to.and_then(project) {
        let hold = WEAPONS[ar.current].hold;
        let from = egui::pos2(c.x + hold.x * screen.height() * 0.95, c.y - (hold.y + 0.02) * screen.height() * 1.1);
        let (wide, core) = if WEAPONS[ar.current].kind == Kind::PhysGun { (egui::Color32::from_rgba_unmultiplied(60, 160, 255, 70), egui::Color32::from_rgba_unmultiplied(190, 235, 255, 230)) } else { (egui::Color32::from_rgba_unmultiplied(255, 150, 40, 40), egui::Color32::from_rgba_unmultiplied(255, 215, 150, 120)) };
        // A sagging curve, as the physics gun's beam bends.
        let mid = from.lerp(to, 0.5) + egui::vec2(0.0, 18.0 * scale);
        let points: Vec<egui::Pos2> = (0..=16).map(|i| i as f32 / 16.0).map(|t| from.lerp(mid, t).lerp(mid.lerp(to, t), t)).collect();
        p.add(egui::Shape::line(points.clone(), egui::Stroke::new(7.0 * scale, wide)));
        p.add(egui::Shape::line(points, egui::Stroke::new(2.0 * scale, core)));
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
