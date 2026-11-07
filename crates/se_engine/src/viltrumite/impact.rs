//! Impacts (spec F11, step 8): the crater pool, the dust pool, the punches, the ground punch, the thunderclap, the sea's splash and the
//! camera's shake.
//!
//! - **Actions** (`Action`, `Hero.action`): the left button punches (the Mixamo punches on foot, `flying_grab_punching` in the air,
//!   layered over the body with the upper-body mask, a 5 m ray for the hit) or, aimed 45 degrees or more down within 15 m of the ground,
//!   punches the ground (`ground_punch`: down to it by its `contact`, a 6 m crater). The right button held charges the thunderclap and
//!   letting go claps (`thunderclap`, `thunderclap_flying`: held at `hold`, the clap at `clap`): a ring of 30 to 150 m by the charge, dust
//!   kicked up when over the ground; it breaks nothing. `start` reads the buttons before the frame's keys (a ground punch or a clap holds
//!   them: `Action::locks`), `act` runs the clock after his step. Their layer is the sampler's slot 4 (`ActLayer`).
//! - **Blasts** (`Hero.blasts`, a fixed list drained each frame): what an impact leaves, for `update_impacts`: a crater with its dust, dust
//!   alone (a punch into the ground, the sea's splash, the clap's ring of dust), a shake.
//! - **The pools** (`Impacts`, made on first enable): 32 craters, each kept in its body's frame (as terrain.rs keeps its nodes) and freed
//!   once he and the camera are both `CRATER_FREE_M` away (D3: no fade before, or one would be seen coming back), the farthest giving way
//!   when full; 64 stateless bursts of dust, the farthest giving way. Each frame they're written as records (`fx.rs`), drawn by `fx_decals`
//!   and `fx_pass`. A crater's middle follows the ground as it's drawn now, so a finer node moves it with the ground.
//! - **Rims:** `props/nibsuper_ground_break1` (broken ground, 4.8 m across) on every crater within `RIM_DRAW_M`, scaled to it, half-sunk,
//!   one `RimMaterial` for all of them (`hero_rim.wesl`), lit by his own light record as the ground's objects are, so cloud shadow and
//!   night are in it, and nothing in it changes a frame. (Not the spec's `HullMaterial`: that takes the stars' light alone, and under
//!   rain clouds its rocks stood sunlit on grey ground.)
//! - **The aftermath:** turning the mod off leaves the pools draining (F3): `ModControl::aftermath` says they hold something, and these
//!   systems run while he's on or it is (C22). Nothing of this exists until the mod's first enable.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{Affine3A, DQuat, DVec3};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use bevy::prelude::*;
use se_core::catalog::ObjectRef;
use se_core::hero_asset::{ClipSet, HeroModel};

use crate::camera::{FlyCamera, MainCamera};
use crate::mods::{HeroAssets, ModControl, ModInput};
use crate::ship::BodyInfo;
use crate::system::SystemBody;
use crate::terrain::Terrain;
use crate::universe::ScaledSpace;
use se_core::upos::UPos;

use super::anim::{Layer, Sampler};
use super::frames::{Frame, Scene};
use super::fx::{CRATERS, DUST_BURSTS, FxCraterGpu, FxDraws, FxDustGpu, FxInput, PARTS, RingShape};
use super::ground::{self, Footing};
use super::hero::State;
use super::{Hero, tune};

/// The action layer's slot in the sampler (`anim.rs`: 4 a masked action).
pub const ACTION_LAYER: usize = 4;

/// What the buttons are doing (F4's Punch, Ground punch, Thunderclap rows).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Action {
    #[default]
    None,
    /// A punch: its clip, seconds into it, when it lands and when it ends; `done` once it has landed.
    Punch { clip: u16, t: f32, hit: f32, end: f32, done: bool },
    /// The ground punch: down to the ground by `contact` from his soles' height `from_m` (m), the crater there, the clip played out.
    Smash { clip: u16, t: f32, contact: f32, end: f32, from_m: f64, done: bool },
    /// The right button held: the clip to its `hold` and held there; `t` the seconds held.
    Charge { clip: u16, t: f32, hold: f32, clap: f32, end: f32 },
    /// Let go: the clip from its hold on, the clap at `clap` (clip seconds), `charge` 0..1.
    Clap { clip: u16, t: f32, clap: f32, end: f32, charge: f32, done: bool },
}

impl Action {
    /// A ground punch and the clap hold his moves: the keys fly and walk nothing, X and Space do nothing.
    pub fn locks(&self) -> bool {
        matches!(self, Action::Smash { .. } | Action::Charge { .. } | Action::Clap { .. })
    }
}

/// The action's layer: it follows the action, then fades out where it stands once the action ends (a charge let go too soon).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActLayer {
    pub clip: Option<u16>,
    pub time: f32,
    pub weight: f32,
    pub mask: u128,
}

/// The actions' clips, found once, and the two masks.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActClips {
    punch: [Option<u16>; 2],
    fly_punch: [Option<u16>; 2],
    smash: Option<u16>,
    clap: Option<u16>,
    clap_flying: Option<u16>,
    /// The spine from `SKEL_Spine0` up, the arms and the head; and all of his own bones (not the cape's: its springs own them).
    upper: u128,
    full: u128,
}

impl ActClips {
    pub fn find(set: &ClipSet, model: &HeroModel) -> ActClips {
        let one = |name: &str| {
            let i = set.find(name).map(|i| i as u16);
            if i.is_none() {
                warn!("viltrumite: no clip '{name}'");
            }
            i
        };
        ActClips {
            punch: [one("punch_cross"), one("punch_hook")],
            fly_punch: [one("flying_grab_punching"), one("flying_grab_punching3")],
            smash: one("ground_punch"),
            clap: one("thunderclap"),
            clap_flying: one("thunderclap_flying"),
            upper: upper_body_mask(model),
            full: full_mask(model),
        }
    }
}

/// Every bone from `SKEL_Spine0` down its children (the spine, the arms, the head), not the cape's.
pub fn upper_body_mask(model: &HeroModel) -> u128 {
    let n = model.bones.len().min(128);
    let cape = model.cape_first.map_or(n, |c| (c as usize).min(n));
    let Some(top) = model.bones.iter().position(|b| b.name == "SKEL_Spine0") else { return 0 };
    let mut mask = 0u128;
    for i in 0..cape {
        // Parents come first: a bone is in if it's the top or its parent is.
        let parent_in = model.bones[i].parent.is_some_and(|p| (p as usize) < 128 && mask >> p & 1 != 0);
        if i == top || parent_in {
            mask |= 1u128 << i;
        }
    }
    mask
}

/// All of his own bones, not the cape's.
fn full_mask(model: &HeroModel) -> u128 {
    let n = model.bones.len().min(128);
    let cape = model.cape_first.map_or(n, |c| (c as usize).min(n));
    if cape >= 128 { u128::MAX } else { (1u128 << cape) - 1 }
}

/// His body for the craters' decal, which leaves him alone (`hero_fx.wesl` `in_him`; he's drawn over the ground it multiplies): a
/// capsule from bone to bone, on past the second by some metres (the hand, the head, the toes), this far round (m). A little wider
/// than the cape's (`cape.rs` `BODY_CAPSULES`): what's left out is darkened like the ground, what's taken in leaves a light rim on
/// the ground round his boots. (One upright cylinder round his middle missed the ground punch's fist, 0.7 m ahead of his feet.)
const BODY_PARTS: [(&str, &str, f32, f32); 14] = [
    ("SKEL_Pelvis", "SKEL_Spine3", 0.0, 0.2),
    ("SKEL_Spine3", "SKEL_Neck_1", 0.0, 0.2),
    ("SKEL_L_UpperArm", "SKEL_R_UpperArm", 0.0, 0.12),
    ("SKEL_Neck_1", "SKEL_Head", 0.16, 0.13),
    ("SKEL_L_UpperArm", "SKEL_L_Forearm", 0.0, 0.085),
    ("SKEL_R_UpperArm", "SKEL_R_Forearm", 0.0, 0.085),
    ("SKEL_L_Forearm", "SKEL_L_Hand", 0.14, 0.075),
    ("SKEL_R_Forearm", "SKEL_R_Hand", 0.14, 0.075),
    ("SKEL_L_Thigh", "SKEL_L_Calf", 0.0, 0.11),
    ("SKEL_R_Thigh", "SKEL_R_Calf", 0.0, 0.11),
    ("SKEL_L_Calf", "SKEL_L_Foot", 0.0, 0.08),
    ("SKEL_R_Calf", "SKEL_R_Foot", 0.0, 0.08),
    ("SKEL_L_Foot", "SKEL_L_Toe0", 0.07, 0.07),
    ("SKEL_R_Foot", "SKEL_R_Toe0", 0.07, 0.07),
];
/// The cape: its two side chains and its middle one, each in two pieces (top to middle, middle to hem), this far round (m).
const CAPE_PART_M: f32 = 0.14;

/// His body as capsules round his bones, found once from the model and placed each frame from the pose he's drawn in (`update_hero`),
/// for the craters' decal (`fx.rs` `FxRecordsGpu::parts`).
pub struct BodyParts {
    /// Bone, bone, on past it (m), radius (m).
    bones: Vec<(u16, u16, f32, f32)>,
    found: bool,
    /// This frame's: two points from the camera each (m), the first's w the radius.
    pub points: [Vec4; PARTS * 2],
    pub count: u32,
}

impl Default for BodyParts {
    fn default() -> Self {
        BodyParts { bones: Vec::new(), found: false, points: [Vec4::ZERO; PARTS * 2], count: 0 }
    }
}

impl BodyParts {
    fn find(model: &HeroModel) -> Vec<(u16, u16, f32, f32)> {
        let bone = |name: &str| model.bones.iter().position(|b| b.name == name).map(|i| i as u16);
        let mut out: Vec<(u16, u16, f32, f32)> = BODY_PARTS.iter().filter_map(|&(a, b, on, r)| Some((bone(a)?, bone(b)?, on, r))).collect();
        if let Some(first) = model.cape_first {
            let (chains, links) = (model.cape_chains as u16, model.cape_links as u16);
            if chains > 0 && links > 1 {
                let mut picked = [0, chains / 2, chains - 1];
                picked.sort_unstable();
                let mut last = None;
                for ch in picked {
                    if last == Some(ch) {
                        continue;
                    }
                    last = Some(ch);
                    let top = first as u16 + ch * links;
                    let mid = top + links / 2;
                    out.push((top, mid, 0.0, CAPE_PART_M));
                    out.push((mid, top + links - 1, 0.0, CAPE_PART_M));
                }
            }
        }
        out.retain(|&(a, b, ..)| (a as usize) < model.bones.len() && (b as usize) < model.bones.len());
        out.truncate(PARTS);
        out
    }

    /// From the pose just sampled, placed where he's drawn (`place`: model space to the camera's frame).
    pub fn place(&mut self, sampler: &Sampler, model: &HeroModel, place: Affine3A) {
        if !self.found {
            self.bones = Self::find(model);
            self.found = true;
            info!("viltrumite: the craters leave {} capsules of him alone", self.bones.len());
        }
        let at = |i: u16| place.transform_point3(Vec3::from(sampler.world(i as usize).translation));
        for (k, &(a, b, on, r)) in self.bones.iter().enumerate() {
            let (pa, pb) = (at(a), at(b));
            let pb = pb + (pb - pa).normalize_or_zero() * on;
            self.points[2 * k] = pa.extend(r);
            self.points[2 * k + 1] = pb.extend(0.0);
        }
        self.count = self.bones.len() as u32;
    }
}

/// A clip's event time by name, or `or`.
fn event(set: &ClipSet, clip: u16, name: &str, or: f32) -> f32 {
    set.clips.get(clip as usize).and_then(|c| set.events_of(c).iter().find(|e| e.name == name)).map_or(or, |e| e.time)
}

fn duration(set: &ClipSet, clip: u16) -> f32 {
    set.clips.get(clip as usize).map_or(1.0, |c| c.duration)
}

/// F11's crater radius from his speed into the ground (m/s).
pub fn crater_radius(speed: f64) -> f64 {
    (tune::CRATER_K * speed.max(0.0).sqrt()).clamp(tune::CRATER_MIN_M, tune::CRATER_MAX_M)
}

/// The splash's size (m) from his speed into the sea.
pub fn splash_size(speed: f64) -> f64 {
    (tune::SPLASH_K * speed.max(0.0).sqrt()).clamp(tune::SPLASH_M.0, tune::SPLASH_M.1)
}

/// How a burst is thrown (`hero_fx.wesl` `dust_vertex`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Up and out: a landing, a ground punch, a punch into the ground.
    Plume = 0,
    /// Out along the ground: the thunderclap's.
    Ring = 1,
    /// Spray falling back as drops, with mist: the sea.
    Spray = 2,
}

/// What an impact leaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlastKind {
    /// A crater of this radius (m) and its dust.
    Crater(f64),
    /// Dust alone, this size (m).
    Dust(f64, Style),
    /// Only the shake.
    Shake,
}

/// An impact, at `local` in `obj`'s frame (anywhere along the line to the ground: it's put on the ground as drawn); `shake_m` its size
/// for the shake (0: none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blast {
    pub obj: ObjectRef,
    pub local: DVec3,
    pub kind: BlastKind,
    pub shake_m: f64,
}

/// This frame's blasts, drained by `update_impacts`: fixed, nothing builds up (more than 8 in a frame are dropped).
#[derive(Clone, Copy, Debug, Default)]
pub struct Blasts {
    list: [Option<Blast>; 8],
}

impl Blasts {
    pub fn push(&mut self, b: Blast) {
        if let Some(slot) = self.list.iter_mut().find(|s| s.is_none()) {
            *slot = Some(b);
        }
    }

    fn take(&mut self) -> [Option<Blast>; 8] {
        std::mem::take(&mut self.list)
    }

    pub fn clear(&mut self) {
        self.list = Default::default();
    }
}

// ---- The actions ------------------------------------------------------------------------------------------------------------------------

/// How high his soles are over land he could punch, if he aims down enough and it's within reach: the ground punch's start (F4).
fn smash_from(hero: &Hero, s: &Scene, fwd: DVec3) -> Option<f64> {
    if matches!(hero.state, State::Land | State::HardLanding) {
        return None;
    }
    let Frame::Attached { obj, local_pos, .. } = hero.frame else { return None };
    let b = s.bodies.iter().find(|b| b.obj == obj)?;
    if (s.footing)(b, local_pos) != Footing::Land {
        return None;
    }
    let g = (s.ground)(b, local_pos);
    g.level?;
    let up = b.orient * local_pos.normalize_or(DVec3::Y);
    if fwd.dot(up) > -tune::GROUND_PUNCH_DEG.to_radians().sin() {
        return None;
    }
    let h = local_pos.length() - g.radial - s.stand_m;
    (h <= tune::GROUND_PUNCH_REACH_M).then_some(h.max(0.0))
}

/// The buttons, before the frame's keys: a punch or the ground punch on the left, the clap's charge on the right, let go to clap.
pub(super) fn start(hero: &mut Hero, input: &ModInput, s: &Scene, fwd: DVec3, travelling: bool, assets: &HeroAssets) {
    if travelling {
        // A Go To carries him: whatever he was doing stops (the layer fades out).
        hero.action = Action::None;
        return;
    }
    let clips = *hero.act_clips.get_or_insert_with(|| ActClips::find(&assets.clips, &assets.model));
    let set = &assets.clips;
    let was = hero.action;
    // Let go of the right button: the clap, if it was held long enough; else nothing. The mouse taken away (Tab's menus, the
    // window's focus lost, a text field) isn't letting go: the charge ends with no clap.
    if let Action::Charge { clip, t, hold, clap, end } = hero.action {
        if input.mouse_lost {
            hero.action = Action::None;
        } else if !input.right_button {
            let (least, most) = tune::THUNDERCLAP_HOLD_S;
            hero.action = if t as f64 >= least {
                // From the pose the charge shows: let go before the clip's hold, it runs on from there, never jumping ahead.
                Action::Clap { clip, t: t.min(hold), clap, end, charge: ((t as f64 - least) / (most - least)).clamp(0.0, 1.0) as f32, done: false }
            } else {
                Action::None
            };
        }
    }
    if input.right_pressed && matches!(hero.action, Action::None | Action::Punch { .. }) {
        let clip = if hero.state.on_foot() { clips.clap } else { clips.clap_flying };
        if let Some(clip) = clip {
            let end = duration(set, clip);
            hero.action = Action::Charge { clip, t: 0.0, hold: event(set, clip, "hold", 0.6), clap: event(set, clip, "clap", 1.23), end };
        }
    }
    let free = match hero.action {
        Action::None => true,
        Action::Punch { t, hit, done, .. } => done && t >= hit + tune::PUNCH_CHAIN_S,
        _ => false,
    };
    if input.left_pressed && free {
        if let (Some(from_m), Some(clip)) = (smash_from(hero, s, fwd), clips.smash) {
            hero.action = Action::Smash { clip, t: 0.0, contact: event(set, clip, "contact", 0.27), end: duration(set, clip), from_m, done: false };
            if !hero.state.on_foot() {
                // He goes down to it: held, the action moves him. (Called before the frame's carry, so the carry holds him too.)
                hero.state = State::Hover;
                hero.short_fall = false;
            }
            hero.frame.set_velocity(s, DVec3::ZERO);
        } else {
            hero.punches = hero.punches.wrapping_add(1);
            let pair = if hero.state.on_foot() { clips.punch } else { clips.fly_punch };
            if let Some(clip) = pair[hero.punches as usize % 2].or(pair[0]) {
                let end = duration(set, clip);
                hero.action = Action::Punch { clip, t: 0.0, hit: event(set, clip, "hit", end * tune::PUNCH_HIT_SHARE), end, done: false };
                hero.sound.punch_swing();
            }
        }
    }
    // A new action over one still showing (a chained punch, a charge over a punch, both buttons at once): from the pose as it
    // stands, never a pop.
    if hero.action != was && !matches!(hero.action, Action::None | Action::Clap { .. }) && hero.act_layer.weight > 0.05 {
        hero.sampler.crossfade(tune::ACTION_FADE_IN_S);
    }
}

/// Where a ray from `from` along `dir` (his body's frame) first goes under what he could stand on, within `reach` (m): the ground or the
/// sea's surface. Marched, then halved.
fn ray_hits(s: &Scene, b: &BodyInfo, from: DVec3, dir: DVec3, reach: f64) -> Option<DVec3> {
    let under = |p: DVec3| {
        let g = (s.ground)(b, p);
        g.level.is_some() && p.length() <= g.radial
    };
    if under(from) {
        return Some(from);
    }
    const STEPS: usize = 20;
    let mut prev = from;
    for i in 1..=STEPS {
        let p = from + dir * (reach * i as f64 / STEPS as f64);
        if under(p) {
            let (mut a, mut c) = (prev, p);
            for _ in 0..8 {
                let m = (a + c) * 0.5;
                if under(m) {
                    c = m;
                } else {
                    a = m;
                }
            }
            return Some(c);
        }
        prev = p;
    }
    None
}

/// The punch lands: the 5 m ray from his shoulders along the aim. On the ground (or the sea) the punch's sound and a puff; else nothing
/// but the swing. (Buildings from step 9.)
fn punch_lands(hero: &mut Hero, s: &Scene, fwd: DVec3) {
    let Frame::Attached { obj, local_pos, .. } = hero.frame else { return };
    let Some(b) = s.bodies.iter().find(|b| b.obj == obj) else { return };
    let from = local_pos + local_pos.normalize_or(DVec3::Y) * tune::PUNCH_FROM_UP_M;
    let dir = b.orient.inverse() * fwd;
    let ground_hit = ray_hits(s, b, from, dir, tune::PUNCH_REACH_M);
    // A building nearer than the ground takes the blow (step 9's second part; step 10 breaks it).
    if let Some((d, i)) = s.solids_on(obj).and_then(|so| so.ray(from, dir, tune::PUNCH_REACH_M)) {
        if ground_hit.is_none_or(|g| (g - from).length() > d) {
            let at = from + dir * d;
            info!("viltrumite: the punch hits a building {d:.1} m from his shoulders");
            hero.sound.punch_hits_building();
            hero.punched = s.solids.map(|so| so.list[i].key);
            hero.blasts.push(Blast { obj, local: at, kind: BlastKind::Dust(tune::PUNCH_DUST_M, Style::Plume), shake_m: tune::PUNCH_DUST_M });
            return;
        }
    }
    let Some(at) = ground_hit else {
        debug!("viltrumite: the punch hits nothing");
        return;
    };
    info!("viltrumite: the punch lands {:.1} m from his shoulders", (at - from).length());
    hero.sound.punch();
    let style = if (s.footing)(b, at) == Footing::Sea { Style::Spray } else { Style::Plume };
    hero.blasts.push(Blast { obj, local: at, kind: BlastKind::Dust(tune::PUNCH_DUST_M, style), shake_m: tune::PUNCH_DUST_M });
}

/// The ground punch's fist reaches the ground: the crater where it lands, ahead of his feet, and he's on them.
fn smash_lands(hero: &mut Hero, s: &Scene) {
    let Frame::Attached { obj, local_pos, .. } = hero.frame else { return };
    let Some(b) = s.bodies.iter().find(|b| b.obj == obj) else { return };
    let up = local_pos.normalize_or(DVec3::Y);
    let ahead = b.orient.inverse() * (hero.rot * DVec3::NEG_Z);
    let flat = (ahead - up * ahead.dot(up)).normalize_or_zero();
    hero.blasts.push(Blast { obj, local: local_pos + flat * tune::SMASH_AHEAD_M, kind: BlastKind::Crater(tune::GROUND_PUNCH_CRATER_M), shake_m: tune::GROUND_PUNCH_CRATER_M });
    hero.sound.ground_punch();
    hero.state = State::Grounded;
    hero.state_t = 0.0;
    hero.frame.set_local_velocity(DVec3::ZERO);
}

/// The clap: its ring from his hands along the aim, by the charge; over the ground a second ring along it, the dust it kicks up and
/// the shockwave's sound. It breaks nothing.
fn clap(hero: &mut Hero, s: &Scene, fwd: DVec3, charge: f64) {
    let lerp = |(a, b): (f64, f64)| a + (b - a) * charge;
    let end_m = lerp(tune::THUNDERCLAP_RING_M);
    hero.sound.thunderclap(charge as f32);
    // (Where he is now, against this frame's bodies: `Hero.world` is last frame's, and the body has moved since, 500 m a frame for Earth.)
    let pos = hero.frame.world(s, hero.world.0);
    // (Far out no body carries him: the ring is kept in the nearest's frame for the moment it lasts, so a clap in space shows too.)
    let carrier = hero.frame.body().and_then(|o| s.bodies.iter().find(|b| b.obj == o));
    let surface = |b: &&BodyInfo| pos.delta_m(b.pos).length() - b.radius;
    let Some(b) = carrier.or_else(|| s.bodies.iter().filter(|b| b.radius > 0.0).min_by(|x, y| surface(x).total_cmp(&surface(y)))) else { return };
    let local = b.orient.inverse() * pos.delta_m(b.pos);
    let up = local.normalize_or(DVec3::Y);
    let ahead = b.orient.inverse() * fwd;
    // In air its light is the air's; out of it, faint: something to see that he clapped.
    let air = smooth(tune::BOOM_AIR_PA.1, tune::BOOM_AIR_PA.0, hero.flow.air.pressure_pa);
    let show = (lerp(tune::CLAP_RING_STRENGTH) * (tune::AIR_SHOW.0 + (1.0 - tune::AIR_SHOW.0) * air)) as f32;
    let shape = RingShape { start_m: tune::CLAP_RING_START_M, end_m, life_s: lerp(tune::CLAP_RING_LIFE_S), ride_s: 0.0, travel_m: end_m * tune::CLAP_TRAVEL };
    let clock = hero.clock;
    hero.fx.add_ring_shaped(b.obj, local + up * 0.3 + ahead * tune::CLAP_AHEAD_M, ahead, clock, show, shape);
    let g = (s.ground)(b, local);
    let over = local.length() - g.radial - s.stand_m;
    let footing = (s.footing)(b, local);
    if g.level.is_some() && footing != Footing::Nothing && over <= tune::CLAP_GROUND_SHARE * end_m {
        let ground_at = up * g.radial;
        hero.fx.add_ring_shaped(b.obj, ground_at + up * 0.5, up, clock, show * tune::CLAP_GROUND_RING as f32, RingShape { travel_m: 0.0, ..shape });
        let style = if footing == Footing::Sea { Style::Spray } else { Style::Ring };
        hero.blasts.push(Blast { obj: b.obj, local: ground_at, kind: BlastKind::Dust(end_m * tune::CLAP_DUST_SHARE, style), shake_m: end_m * 0.15 });
        hero.sound.shockwave();
    } else {
        hero.blasts.push(Blast { obj: b.obj, local, kind: BlastKind::Shake, shake_m: end_m * 0.1 });
    }
}

/// A layer's weight: in over `fade_in`, out over the last `fade_out` before `end`.
fn ramp(t: f32, end: f32, fade_in: f32, fade_out: f32) -> f32 {
    (t / fade_in).min((end - t) / fade_out).clamp(0.0, 1.0)
}

/// One frame of the action, after his step: its clock, the ground punch's way down and its contact, the punch's hit, the clap; then the
/// layer.
pub(super) fn act(hero: &mut Hero, s: &Scene, fwd: DVec3, dt: f64) {
    let step = dt as f32;
    let clips = hero.act_clips.unwrap_or_default();
    match hero.action {
        Action::None => {}
        Action::Punch { clip, t, hit, end, done } => {
            let t = t + step;
            let landed = !done && t >= hit;
            hero.action = if t >= end { Action::None } else { Action::Punch { clip, t, hit, end, done: done || landed } };
            if landed {
                punch_lands(hero, s, fwd);
            }
        }
        Action::Smash { clip, t, contact, end, from_m, done } => {
            let t = t + step;
            if !done && !hero.state.on_foot() {
                // Down to the ground by the contact, faster and faster. (Started on his feet, his step holds him there.)
                if let Frame::Attached { obj, local_pos, local_vel, .. } = &mut hero.frame {
                    if let Some(b) = s.bodies.iter().find(|b| b.obj == *obj) {
                        let g = (s.ground)(b, *local_pos);
                        let k = (t / contact).min(1.0) as f64;
                        *local_pos = local_pos.normalize_or(DVec3::Y) * (g.radial + s.stand_m + from_m * (1.0 - k * k));
                        *local_vel = DVec3::ZERO;
                    }
                }
            }
            let lands = !done && t >= contact;
            hero.action = if t >= end { Action::None } else { Action::Smash { clip, t, contact, end, from_m, done: done || lands } };
            if lands {
                smash_lands(hero, s);
            }
        }
        Action::Charge { clip, t, hold, clap, end } => hero.action = Action::Charge { clip, t: t + step, hold, clap, end },
        Action::Clap { clip, t, clap: at, end, charge, done } => {
            let t = t + step;
            let claps = !done && t >= at;
            hero.action = if t >= end { Action::None } else { Action::Clap { clip, t, clap: at, end, charge, done: done || claps } };
            if claps {
                clap(hero, s, fwd, charge as f64);
            }
        }
    }
    // The layer.
    let l = &mut hero.act_layer;
    match hero.action {
        Action::Punch { clip, t, end, .. } => *l = ActLayer { clip: Some(clip), time: t, weight: ramp(t, end, tune::PUNCH_FADE_IN_S, tune::PUNCH_FADE_OUT_S), mask: clips.upper },
        Action::Smash { clip, t, end, .. } => {
            *l = ActLayer { clip: Some(clip), time: t, weight: ramp(t, end, tune::ACTION_FADE_IN_S, tune::ACTION_FADE_OUT_S), mask: clips.full }
        }
        Action::Charge { clip, t, hold, .. } => *l = ActLayer { clip: Some(clip), time: t.min(hold), weight: (t / tune::ACTION_FADE_IN_S).min(1.0), mask: clips.full },
        // (The charge it comes from is held at least `THUNDERCLAP_HOLD_S`, past its fade in: no jump.)
        Action::Clap { clip, t, end, .. } => *l = ActLayer { clip: Some(clip), time: t, weight: ((end - t) / tune::ACTION_FADE_OUT_S).clamp(0.0, 1.0), mask: clips.full },
        Action::None => l.weight = (l.weight - step / tune::ACTION_FADE_OUT_S).max(0.0),
    }
    let l = hero.act_layer;
    // Its clock is the action's own: the sampler doesn't move it (rate 0).
    hero.sampler.layers[ACTION_LAYER] = if l.weight > 0.0 { Layer { clip: l.clip, time: l.time, weight: l.weight, mask: l.mask, rate: 0.0 } } else { Layer::default() };
}

/// The mod turned off, or he was put somewhere: no action, no layer.
pub(super) fn reset(hero: &mut Hero) {
    hero.action = Action::None;
    hero.act_layer = ActLayer::default();
    hero.sampler.layers[ACTION_LAYER] = Layer::default();
    hero.blasts.clear();
}

fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---- The pools --------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Crater {
    obj: ObjectRef,
    /// On the line to its middle (body frame), and the ground's radius there as last drawn (m).
    dir: DVec3,
    radial: f64,
    /// The ground's normal there (body frame).
    normal: DVec3,
    radius: f64,
    born: f64,
    seed: f32,
    /// The rim's turn about the normal.
    yaw: f64,
}

#[derive(Clone, Copy, Debug)]
struct Burst {
    obj: ObjectRef,
    local: DVec3,
    normal: DVec3,
    born: f64,
    size: f64,
    life: f64,
    style: Style,
    colour: [f32; 3],
    gravity: f64,
    hold: f64,
    quads: u8,
    seed: f32,
}

#[derive(Clone, Copy, Debug)]
struct Shake {
    born: f64,
    deg: f64,
    phase: f64,
}

/// The rims: one mesh, one material, a fixed entity a crater. Each is placed from the camera by `update_impacts` (its `Transform`,
/// as the floating origin would), not through `UPosition`: that would rewrite all 32 every frame, hidden or not, the mod off too.
struct Rims {
    entities: [Entity; CRATERS],
    shown: [bool; CRATERS],
    /// Frames the last one stays out as a speck at the camera, so its pipeline is built before a crater needs it. Counted while
    /// he's on: turned off before it's done, the speck hides and waits for the next time.
    warm: u32,
    /// The model across (m): 4.8.
    across: f64,
}

#[derive(Component)]
pub struct RimPart;

/// The pools, made on first enable.
#[derive(Resource)]
pub struct Impacts {
    craters: [Option<Crater>; CRATERS],
    bursts: [Option<Burst>; DUST_BURSTS],
    shakes: [Option<Shake>; 4],
    rims: Option<Rims>,
    /// Real seconds since first enable, his clock or not (it runs on in the aftermath).
    clock: f64,
    seeds: u32,
    /// The rotation the shake adds to the camera this frame (radians: yaw, pitch).
    pub shake_now: Vec2,
    /// `hero impacts`: log the pools next frame.
    pub log: bool,
    /// In the aftermath, where the light the rims and the bowls take is worked out (draw.rs `aftermath_light`): the crater (or the
    /// dust) nearest the camera. `None` while he's on (his own light) or with the pools empty.
    light_spot: Option<UPos>,
}

impl Impacts {
    pub fn light_spot(&self) -> Option<UPos> {
        self.light_spot
    }

    /// The mod turned off: the warm-up's speck hides unless a crater uses its rim. (With the pools empty these systems stop at
    /// once, and a speck left showing would be drawn every frame with the mod off.)
    pub fn hide_warm_rim(&self, vis: &mut Query<&mut Visibility, With<RimPart>>) {
        let Some(rim) = self.rims.as_ref().filter(|r| !r.shown[CRATERS - 1]) else { return };
        if let Ok(mut v) = vis.get_mut(rim.entities[CRATERS - 1]) {
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
                info!("viltrumite: the rims' warm-up hidden with the mod, {} frames of it left", rim.warm);
            }
        }
    }

    fn new_seed(&mut self) -> f32 {
        self.seeds = self.seeds.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seeds >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Anything in the pools.
    pub fn live(&self) -> bool {
        self.craters.iter().any(Option::is_some) || self.bursts.iter().any(Option::is_some)
    }

    pub fn crater_count(&self) -> usize {
        self.craters.iter().flatten().count()
    }
}

/// The slot to use: a free one, else the farthest from the camera.
fn slot_for<T>(pool: &[Option<T>], far: impl Fn(&T) -> f64) -> usize {
    if let Some(i) = pool.iter().position(Option::is_none) {
        return i;
    }
    (0..pool.len()).max_by(|&a, &b| far(pool[a].as_ref().unwrap()).total_cmp(&far(pool[b].as_ref().unwrap()))).unwrap_or(0)
}

/// The live crater a new one of `radius` m at `on` (`obj`'s frame) lands in: middles within `CRATER_MERGE` of the larger radius.
fn crater_under(craters: &[Option<Crater>], obj: ObjectRef, on: DVec3, radius: f64) -> Option<usize> {
    craters.iter().position(|x| x.is_some_and(|x| x.obj == obj && (x.dir * x.radial - on).length() < tune::CRATER_MERGE * x.radius.max(radius)))
}

/// A shake of an impact `size` m across, `from_cam` m from the camera (none for size 0).
fn push_shake(im: &mut Impacts, size: f64, from_cam: f64, clock: f64) {
    if size <= 0.0 {
        return;
    }
    let deg = tune::SHAKE_MAX_DEG * (tune::SHAKE_K * size / (from_cam + size)).min(1.0);
    let phase = im.new_seed() as f64 * 100.0;
    let i = slot_for(&im.shakes, |s: &Shake| -s.born);
    im.shakes[i] = Some(Shake { born: clock, deg, phase });
}

/// A texture of the mod's (`props/`, `sprites/`), or a neutral colour.
fn prop_texture(name: &str, srgb: bool, neutral: [u8; 4]) -> Option<Image> {
    let read = || -> Result<Image, String> {
        let path = crate::mods::mod_file("viltrumite", name).ok_or_else(|| "no file".to_string())?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let info = se_core::dds::parse_dds(&bytes).map_err(|e| e.to_string())?;
        crate::mr_assets::dds_image(&bytes, &info, srgb)
    };
    match read() {
        Ok(i) => Some(i),
        Err(e) => {
            warn!("viltrumite: {name}: {e}: a flat colour instead");
            Some(crate::ship_assets::image_with_mips(neutral.to_vec(), 1, 1, if srgb { bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb } else { bevy::render::render_resource::TextureFormat::Rgba8Unorm }))
        }
    }
}

/// The rim's mesh in metres: `nibsuper_ground_break1` (a unit box in its file, scaled by its own size), half of it under the ground, its
/// uv the model's own (`v + 5000` in the file, layer 0: step 2's Build log); and how far across it is (m).
fn rim_mesh() -> Option<(Mesh, f64)> {
    let path = crate::mods::mod_file("viltrumite", "props/nibsuper_ground_break1.sehm")?;
    let ((p, n, uv, idx), size) = crate::scatter::library::sehm_from_bytes(&std::fs::read(path).ok()?)?;
    let pos: Vec<[f32; 3]> = p.iter().map(|v| [v[0] * size[0], (v[1] - tune::RIM_SINK as f32) * size[1], v[2] * size[2]]).collect();
    let uvs: Vec<[f32; 2]> = uv.iter().map(|t| [t[0], t[1] - 5000.0]).collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, n)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(idx));
    if let Err(e) = mesh.generate_tangents() {
        warn!("viltrumite: the rim's tangents: {e}");
    }
    Some((mesh, size[0].max(size[2]) as f64))
}

/// The rims' material: the rock's textures and colour, his frame and light records (`hero_rim.wesl`). Made once; never changed.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct RimMaterial {
    #[storage(0, read_only)]
    pub frame: Handle<ShaderBuffer>,
    #[storage(1, read_only)]
    pub light: Handle<ShaderBuffer>,
    #[texture(2)]
    #[sampler(3)]
    pub diffuse: Handle<Image>,
    #[texture(4)]
    #[sampler(5)]
    pub normal: Handle<Image>,
    #[uniform(6)]
    pub tint: Vec4,
}

impl Material for RimMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/hero_rim.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/hero_rim.wesl".into()
    }
    fn specialize(_pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef, _key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_TANGENT.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
    }
}

/// First enable, once the hero is made: the pools, the rims (hidden), the dust's sprites.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_impacts(
    mut commands: Commands,
    control: Res<ModControl>,
    draws: Res<super::draw::HeroDraws>,
    mut input: ResMut<FxInput>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut rim_mats: ResMut<Assets<RimMaterial>>,
) {
    let started = std::time::Instant::now();
    if let Some(sprite) = prop_texture("sprites/ptfx_smoke_wispy_anim.dds", false, [255, 255, 255, 128]) {
        input.sprite = images.add(sprite);
    }
    let rims = rim_mesh().map(|(mesh, across)| {
        let mesh = meshes.add(mesh);
        let mut image = |name: &str, srgb: bool, neutral: [u8; 4]| images.add(prop_texture(name, srgb, neutral).expect("a texture or its neutral colour"));
        let material = rim_mats.add(RimMaterial {
            frame: draws.frame_buffer(),
            light: input.light.clone(),
            diffuse: image("props/rock.dds", true, [128, 118, 104, 255]),
            normal: image("props/rock_N.dds", false, [128, 128, 255, 255]),
            tint: Vec3::from_array(tune::RIM_TINT).extend(1.0),
        });
        let entities = std::array::from_fn(|i| {
            // (The last one starts out as a speck at the camera while he's on, so its pipeline is built before the first crater.)
            let warm = i == CRATERS - 1 && control.hero;
            commands
                .spawn((
                    Name::new("Viltrumite crater rim"),
                    RimPart,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    warm_speck(),
                    if warm { Visibility::Visible } else { Visibility::Hidden },
                    NoFrustumCulling,
                ))
                .id()
        });
        Rims { entities, shown: [false; CRATERS], warm: tune::RIM_WARM_FRAMES, across }
    });
    if rims.is_none() {
        warn!("viltrumite: the crater rim's model can't be read: craters without rims");
    }
    commands.insert_resource(Impacts { craters: [None; CRATERS], bursts: [None; DUST_BURSTS], shakes: [None; 4], rims, clock: 0.0, seeds: 0x9e37_79b9, shake_now: Vec2::ZERO, log: false, light_spot: None });
    info!("viltrumite: impacts made in {} ms", started.elapsed().as_millis());
}

/// A crater's record of the sea (`FxCraterGpu::sea`): the sea's up there (world axes) and how high the crater's middle stands over
/// it (m), so the decal leaves the water alone; on a body with no sea, a height nothing reaches.
fn sea_record(b: &BodyInfo, dir: DVec3, radial: f64) -> Vec4 {
    if !b.terrain.is_some_and(|t| t.has_ocean) {
        return Vec4::new(0.0, 1.0, 0.0, 1e9);
    }
    // (As `ground_on`: the sea's surface is the body's radius along the ellipsoid, the ground's `radial` the same line's.)
    let k = ((dir / b.shape).normalize_or(DVec3::Y) * b.shape).length();
    // The ellipsoid's normal, not the line to the middle: on a flattened Earth they part by up to 0.2 degrees, 0.3 m over 100 m.
    let n = b.orient * (dir / (b.shape * b.shape)).normalize_or(dir);
    n.as_vec3().extend((radial - b.radius * k) as f32)
}

/// The warm-up's rim: a speck at the camera (inside the near plane, but queued, so its pipeline gets built).
fn warm_speck() -> Transform {
    Transform::from_scale(Vec3::splat(1e-5))
}

/// One frame of the pools: this frame's blasts in, the freed out, the records written, the rims placed, the shake worked out. While he's
/// on or the pools hold something.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_impacts(
    time: Res<Time<Real>>,
    (mut control, cam): (ResMut<ModControl>, Res<FlyCamera>),
    mut hero: ResMut<Hero>,
    mut impacts: ResMut<Impacts>,
    (mut fx, mut input): (ResMut<FxDraws>, ResMut<FxInput>),
    bodies: Query<(&SystemBody, Option<&Terrain>)>,
    space: Res<ScaledSpace>,
    mut rims: Query<(&mut Transform, &mut Visibility), With<RimPart>>,
) {
    let im = &mut *impacts;
    let dt = time.delta_secs_f64().min(0.1);
    im.clock += dt;
    let clock = im.clock;
    let info = |obj: ObjectRef| {
        bodies.iter().find(|(b, _)| b.obj == obj).map(|(b, t)| (BodyInfo { obj: b.obj, pos: b.pos, orient: b.orient, radius: b.radius_m, shape: b.shape, gm: 0.0, terrain: t }, ()))
    };
    let ground = |b: &BodyInfo, local: DVec3| ground::support_height(b, local, None, 0.0);
    let world = |b: &BodyInfo, local: DVec3| b.pos.offset_m(b.orient * local);

    // This frame's blasts.
    let blasts = if control.hero { hero.blasts.take() } else { Default::default() };
    for blast in blasts.into_iter().flatten() {
        let Some((b, _)) = info(blast.obj) else { continue };
        if blast.kind == BlastKind::Shake {
            // Where it is, not on the ground below: a clap in the air shakes the camera beside him.
            push_shake(im, blast.shake_m, world(&b, blast.local).delta_m(cam.pos).length(), clock);
            continue;
        }
        let dir = blast.local.normalize_or(DVec3::Y);
        let radial = ground(&b, blast.local).radial;
        let at = dir * radial;
        let span = match blast.kind {
            BlastKind::Crater(r) => r.clamp(2.0, 12.0),
            _ => 2.0,
        };
        let normal = ground::ground_triangle(|p| ground(&b, p), at, span).map_or(dir, |[p0, p1, p2]| {
            let n = (p1 - p0).cross(p2 - p0).normalize_or(dir);
            if n.dot(dir) < 0.0 { -n } else { n }
        });
        let here = world(&b, at);
        let from_cam = here.delta_m(cam.pos).length();
        let airy = b.terrain.is_some_and(|t| t.atmosphere.is_some());
        let gravity = (hero.readout.gravity_g * 9.806_65).max(0.05);
        let pressure = hero.flow.air.pressure_pa;
        let dust = |im: &mut Impacts, size: f64, style: Style| {
            let hold = if pressure > tune::BOOM_AIR_PA.1 { (0.3 + 0.06 * size) * (pressure / 1e5).clamp(1e-3, 10.0).powf(0.15) } else { 0.0 };
            let throw = size * if style == Style::Spray { 2.6 } else { 2.0 };
            let flight = (2.0 * throw / gravity).clamp(2.0, tune::DUST_VACUUM_MAX_S);
            let life = match style {
                Style::Spray => flight.min(12.0).max(2.5),
                // (A ring of dust along the ground settles sooner: it's thin.)
                Style::Ring if hold > 0.0 => 0.6 * (tune::DUST_LIFE_S.0 + (tune::DUST_LIFE_S.1 - tune::DUST_LIFE_S.0) * (size / 60.0).clamp(0.0, 1.0)),
                _ if hold > 0.0 => tune::DUST_LIFE_S.0 + (tune::DUST_LIFE_S.1 - tune::DUST_LIFE_S.0) * (size / 60.0).clamp(0.0, 1.0),
                _ => flight,
            };
            let colour = match style {
                Style::Spray => tune::DUST_SPRAY,
                _ if airy => tune::DUST_LAND,
                _ => tune::DUST_AIRLESS,
            };
            let quads = (tune::DUST_QUADS.0 as f64 + size * tune::DUST_QUADS.1).clamp(tune::DUST_QUADS.0 as f64, 64.0) as u8;
            let seed = im.new_seed();
            let i = slot_for(&im.bursts, |x: &Burst| info(x.obj).map_or(f64::INFINITY, |(xb, _)| world(&xb, x.local).delta_m(cam.pos).length()));
            im.bursts[i] = Some(Burst { obj: b.obj, local: at, normal, born: clock, size, life, style, colour, gravity, hold, quads, seed });
        };
        match blast.kind {
            BlastKind::Crater(radius) => {
                // Over a crater already there (ground punches where he stands): that one, grown if this is bigger. Each crater
                // multiplies the scene, so two in one place would darken it twice, and their rims would pile up.
                if let Some(i) = crater_under(&im.craters, b.obj, dir * radial, radius) {
                    let c = im.craters[i].as_mut().expect("just found");
                    if radius > c.radius {
                        (c.dir, c.radial, c.normal, c.radius) = (dir, radial, normal, radius);
                    }
                    info!("viltrumite: crater {i} hit again, now {:.1} m, {:.0} m from the camera", c.radius, from_cam);
                } else {
                    let seed = im.new_seed();
                    let i = slot_for(&im.craters, |x: &Crater| info(x.obj).map_or(f64::INFINITY, |(xb, _)| world(&xb, x.dir * x.radial).delta_m(cam.pos).length()));
                    im.craters[i] = Some(Crater { obj: b.obj, dir, radial, normal, radius, born: clock, seed, yaw: seed as f64 * std::f64::consts::TAU });
                    info!("viltrumite: crater {i}, {radius:.1} m, {:.0} m from the camera", from_cam);
                }
                dust(im, radius * tune::DUST_OF_CRATER, Style::Plume);
            }
            BlastKind::Dust(size, style) => dust(im, size, style),
            BlastKind::Shake => {}
        }
        push_shake(im, blast.shake_m, from_cam, clock);
    }

    // The freed: a body gone, both he and the camera far enough away (in the aftermath the camera alone: he isn't there), a burst's life over.
    let him = if control.hero { Some(hero.world.0) } else { None };
    for (i, slot) in im.craters.iter_mut().enumerate() {
        let Some(c) = slot else { continue };
        let Some((b, _)) = info(c.obj) else {
            info!("viltrumite: crater {i} freed: its body is gone");
            *slot = None;
            continue;
        };
        let at = world(&b, c.dir * c.radial);
        let d_cam = at.delta_m(cam.pos).length();
        let d_him = him.map_or(d_cam, |p| at.delta_m(p).length());
        if d_cam > tune::CRATER_FREE_M && d_him > tune::CRATER_FREE_M {
            info!("viltrumite: crater {i} freed: {:.2} km from the camera, {:.2} km from him", d_cam / 1e3, d_him / 1e3);
            *slot = None;
        }
    }
    for slot in im.bursts.iter_mut() {
        if slot.is_some_and(|x| clock - x.born >= x.life || info(x.obj).is_none()) {
            *slot = None;
        }
    }
    let live = im.live();
    if control.aftermath != live {
        control.aftermath = live;
    }

    // The records, from the camera, the ground as it's drawn now. (And the nearest of them, for the aftermath's light.)
    let r = &mut *fx.records;
    let mut crater_bits = 0u32;
    let mut spot: Option<(f64, UPos)> = None;
    let mut nearer = |at: UPos| {
        let d = at.delta_m(cam.pos).length();
        if spot.is_none_or(|(e, _)| d < e) {
            spot = Some((d, at));
        }
    };
    for (i, slot) in im.craters.iter_mut().enumerate() {
        r.craters[i] = FxCraterGpu::default();
        let Some(c) = slot else { continue };
        let Some((b, _)) = info(c.obj) else { continue };
        let g = ground(&b, c.dir * c.radial);
        if g.level.is_some() {
            c.radial = g.radial;
        }
        let at = world(&b, c.dir * c.radial);
        nearer(at);
        let age = clock - c.born;
        r.craters[i] = FxCraterGpu {
            at: at.delta_m(cam.pos).as_vec3().extend(c.radius as f32),
            up: (b.orient * c.normal).as_vec3().extend(c.seed),
            look: Vec4::new(smooth(0.0, tune::CRATER_IN_S, age) as f32, tune::CRATER_DEPTH, tune::CRATER_LIP, tune::CRATER_REACH),
            sea: sea_record(&b, c.dir, c.radial),
        };
        crater_bits |= 1 << i;
    }
    let mut quads = [0u8; DUST_BURSTS];
    for (i, slot) in im.bursts.iter().enumerate() {
        r.dust[i] = FxDustGpu::default();
        let Some(x) = slot else { continue };
        let Some((b, _)) = info(x.obj) else { continue };
        nearer(world(&b, x.local));
        r.dust[i] = FxDustGpu {
            at: world(&b, x.local).delta_m(cam.pos).as_vec3().extend((clock - x.born) as f32),
            up: (b.orient * x.normal).as_vec3().extend(x.size as f32),
            look: Vec3::from_array(x.colour).extend(x.seed),
            motion: Vec4::new(x.life as f32, x.gravity as f32, x.hold as f32, x.style as u32 as f32),
        };
        quads[i] = x.quads;
    }
    if input.craters != crater_bits || input.dust != quads {
        input.craters = crater_bits;
        input.dust = quads;
    }
    im.light_spot = if control.hero { None } else { spot.map(|(_, at)| at) };

    // The rims: within reach of the camera, scaled to their crater, on the ground as it's drawn now, half-sunk (the mesh is). Placed
    // from the camera as the floating origin places what it moves (the body's scale for the compressed space).
    if let Some(rim) = im.rims.as_mut() {
        let across = rim.across;
        if control.hero && rim.warm > 0 {
            rim.warm -= 1;
        }
        for (i, slot) in im.craters.iter().enumerate() {
            let want = slot.and_then(|c| {
                let (b, _) = info(c.obj)?;
                let at = world(&b, c.dir * c.radial);
                (at.delta_m(cam.pos).length() < tune::RIM_DRAW_M + c.radius).then_some((b, c, at))
            });
            let Ok((mut tf, mut vis)) = rims.get_mut(rim.entities[i]) else { continue };
            rim.shown[i] = want.is_some();
            let show = match want {
                Some((b, c, at)) => {
                    let k = space.factor(b.pos.delta_m(cam.pos).length());
                    // Across with the crater, up by the square root of that: uniformly, a big crater's slabs would bury him.
                    let s = 2.0 * c.radius * tune::RIM_ACROSS_SHARE / across;
                    *tf = Transform {
                        translation: (at.delta_m(cam.pos) * k).as_vec3(),
                        rotation: (b.orient * DQuat::from_rotation_arc(DVec3::Y, c.normal) * DQuat::from_rotation_y(c.yaw)).as_quat(),
                        scale: (DVec3::new(s, s.sqrt(), s) * k).as_vec3(),
                    };
                    true
                }
                None if i == CRATERS - 1 && control.hero && rim.warm > 0 => {
                    if *tf != warm_speck() {
                        *tf = warm_speck();
                    }
                    true
                }
                None => false,
            };
            let v = if show { Visibility::Visible } else { Visibility::Hidden };
            if *vis != v {
                *vis = v;
            }
        }
    }

    // The shake: each dying out, at most the most in all.
    let mut shake = Vec2::ZERO;
    for slot in im.shakes.iter_mut() {
        let Some(s) = slot else { continue };
        let age = clock - s.born;
        let k = (-age / tune::SHAKE_TAU_S).exp();
        if k < 0.01 {
            *slot = None;
            continue;
        }
        let w = std::f64::consts::TAU * tune::SHAKE_HZ;
        let a = (s.deg * k).to_radians();
        shake += Vec2::new(((age * w + s.phase).sin() * a) as f32, ((age * w * 1.37 + s.phase * 1.9).sin() * a * 0.8) as f32);
    }
    let most = tune::SHAKE_MAX_DEG.to_radians() as f32;
    im.shake_now = if shake.length() > most { shake * (most / shake.length()) } else { shake };
    if !control.hero {
        im.shake_now = Vec2::ZERO;
    }

    if std::mem::take(&mut im.log) {
        for (i, c) in im.craters.iter().enumerate() {
            let Some(c) = c else { continue };
            let Some((b, _)) = info(c.obj) else { continue };
            let at = world(&b, c.dir * c.radial);
            info!(
                "hero impacts: crater {i}: {:.1} m, {:.0} s old, {:.3} km from the camera, {:.3} km from him, rim shown {}",
                c.radius,
                clock - c.born,
                at.delta_m(cam.pos).length() / 1e3,
                him.map_or(f64::NAN, |p| at.delta_m(p).length() / 1e3),
                im.rims.as_ref().is_some_and(|r| r.shown[i])
            );
        }
        let bursts: Vec<String> = im
            .bursts
            .iter()
            .flatten()
            .map(|x| {
                let d = info(x.obj).map_or(f64::NAN, |(b, _)| world(&b, x.local).delta_m(cam.pos).length());
                format!("{:?} {:.1} m {:.1}/{:.1} s, {} quads, held {:.2} s, {d:.0} m from the camera", x.style, x.size, clock - x.born, x.life, x.quads, x.hold)
            })
            .collect();
        info!("hero impacts: {} craters, {} bursts {bursts:?}, aftermath {}, shake {:?}", im.crater_count(), bursts.len(), control.aftermath, im.shake_now);
    }
}

/// F7's shake: a rotation of the view alone, after its transform is set from the camera, never written back to the camera. The
/// clouds' view takes the same turn (`ViewShake`, read only in a frame it's written), or they'd hold still while the scene shook.
pub(super) fn apply_shake(impacts: Res<Impacts>, mut q: Query<&mut Transform, With<MainCamera>>, mut view: ResMut<crate::volumetric_clouds::ViewShake>) {
    let s = impacts.shake_now;
    if s == Vec2::ZERO {
        return;
    }
    let turn = Quat::from_euler(EulerRot::YXZ, s.x, s.y, 0.0);
    view.0 = turn.as_dquat();
    for mut tf in &mut q {
        tf.rotation = (tf.rotation * turn).normalize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crater_grows_with_the_speed_within_its_limits() {
        assert_eq!(crater_radius(0.0), tune::CRATER_MIN_M);
        assert_eq!(crater_radius(35.0), tune::CRATER_MIN_M.max(0.4 * 35f64.sqrt()));
        assert!((crater_radius(900.0) - 12.0).abs() < 1e-9);
        assert_eq!(crater_radius(30_000.0), tune::CRATER_MAX_M);
        assert!(splash_size(2.0) < splash_size(500.0) && splash_size(1e9) == tune::SPLASH_M.1);
    }

    #[test]
    fn a_full_pool_gives_up_its_farthest() {
        let pool = [Some(5.0), Some(50.0), Some(7.0)];
        assert_eq!(slot_for(&pool, |&d| d), 1);
        let pool = [Some(5.0), None, Some(7.0)];
        assert_eq!(slot_for(&pool, |&d| d), 1);
    }

    #[test]
    fn blasts_never_build_up() {
        let mut b = Blasts::default();
        for i in 0..20 {
            b.push(Blast { obj: ObjectRef::Body(0), local: DVec3::X * i as f64, kind: BlastKind::Shake, shake_m: 1.0 });
        }
        let taken = b.take();
        assert_eq!(taken.iter().flatten().count(), 8);
        assert_eq!(b.take().iter().flatten().count(), 0);
    }

    /// Ground punches where he stands land in the crater already there; one beside it doesn't.
    #[test]
    fn a_crater_on_a_crater_is_the_same_crater() {
        let c = |x: f64, radius: f64| Crater { obj: ObjectRef::Body(3), dir: DVec3::new(x, 0.0, 6.4e6).normalize(), radial: 6.4e6, normal: DVec3::Z, radius, born: 0.0, seed: 0.5, yaw: 0.0 };
        let pool = [None, Some(c(0.0, 6.0)), Some(c(100.0, 6.0))];
        let at = |x: f64| DVec3::new(x, 0.0, 6.4e6).normalize() * 6.4e6;
        assert_eq!(crater_under(&pool, ObjectRef::Body(3), at(0.7), 6.0), Some(1));
        assert_eq!(crater_under(&pool, ObjectRef::Body(3), at(97.0), 2.6), Some(2));
        // Beside it, on another body, or a small one off the edge of a small one: a crater of its own.
        assert_eq!(crater_under(&pool, ObjectRef::Body(3), at(30.0), 6.0), None);
        assert_eq!(crater_under(&pool, ObjectRef::Body(4), at(0.0), 6.0), None);
        assert_eq!(crater_under(&pool, ObjectRef::Body(3), at(105.0), 2.0), None);
        // A big one over a small one takes it in.
        assert_eq!(crater_under(&pool, ObjectRef::Body(3), at(8.0), 17.0), Some(1));
    }

    /// The capsules the craters leave alone cover his limbs, his head, his hands past the wrist and the cape, from the real model.
    #[test]
    fn the_craters_leave_all_of_him_alone() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/viltrumite");
        let Ok(b) = std::fs::read(dir.join("omniman.sevm")) else { return };
        let m = se_core::hero_asset::parse_model(&b).unwrap();
        let parts = BodyParts::find(&m);
        assert_eq!(parts.len(), BODY_PARTS.len() + 6, "every bone found, and the cape's three chains in two pieces");
        assert!(parts.len() <= PARTS);
        let cape = m.cape_first.unwrap() as u16;
        assert!(parts.iter().filter(|p| p.0 >= cape).all(|p| p.1 > p.0 && p.3 == CAPE_PART_M));
    }

    #[test]
    fn a_layer_comes_in_and_goes_out() {
        assert_eq!(ramp(0.0, 1.0, 0.1, 0.2), 0.0);
        assert_eq!(ramp(0.5, 1.0, 0.1, 0.2), 1.0);
        assert!((ramp(0.9, 1.0, 0.1, 0.2) - 0.5).abs() < 1e-5);
        assert_eq!(ramp(1.0, 1.0, 0.1, 0.2), 0.0);
    }

    #[test]
    fn crater_freeing_uses_the_buildings_far_range_whatever_the_setting() {
        assert_eq!(tune::CRATER_FREE_M, crate::scatter::FAR_RANGE_M + 500.0);
        assert_eq!(tune::CRATER_FREE_M, 20_500.0);
    }

    /// The masks from the real model: the punch moves his spine, arms and head, not his legs; the full one all of him, not the cape.
    #[test]
    fn the_upper_body_mask_is_the_upper_body() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/viltrumite");
        let Ok(b) = std::fs::read(dir.join("omniman.sevm")) else { return };
        let m = se_core::hero_asset::parse_model(&b).unwrap();
        let mask = upper_body_mask(&m);
        let has = |name: &str| m.bones.iter().position(|b| b.name == name).is_some_and(|i| mask >> i & 1 != 0);
        for b in ["SKEL_Spine0", "SKEL_Spine3", "SKEL_R_Hand", "SKEL_L_Forearm", "SKEL_Head", "SKEL_Neck_1"] {
            assert!(has(b), "{b}");
        }
        for b in ["SKEL_ROOT", "SKEL_Pelvis", "SKEL_L_Thigh", "SKEL_R_Foot", "SKEL_Spine_Root"] {
            assert!(!has(b), "{b}");
        }
        let full = full_mask(&m);
        let cape = m.cape_first.unwrap() as usize;
        assert!(full >> (cape - 1) & 1 != 0 && full >> cape & 1 == 0);
    }
}
