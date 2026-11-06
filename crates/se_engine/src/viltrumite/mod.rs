//! The Viltrumite powers mod (docs/spec-mods-viltrumite-powers.md): a playable Omni-Man. The menu, the registry
//! and the files' checks are `crate::mods`. So far (step 3): he is drawn, lit as the ground and plays `bf_idle`,
//! hovering in front of the camera; the camera stays free. Step 4 brings flight and the real frames (`Hero`'s
//! `Place` is the simple version of them, and the dev commands' `here`, `stand` and `face` go with it).

pub mod anim;
pub mod draw;
pub mod tune;

use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use se_core::upos::UPos;

use crate::camera::FlyCamera;
use crate::mods::{HeroAssets, ModControl};
use crate::system::SystemBody;
use crate::terrain::Terrain;
use crate::universe::UniverseSet;

use anim::Sampler;
use draw::HeroDraws;

pub struct ViltrumitePlugin;

impl Plugin for ViltrumitePlugin {
    fn build(&self, app: &mut App) {
        draw::build(app);
        app.init_resource::<Hero>()
            // First enable: `HeroAssets` arrives from the Mods menu's first enable, and these make the rest.
            .add_systems(Update, draw::spawn_hero.run_if(first_enable))
            .add_systems(Update, draw::sync_visibility.run_if(resource_exists::<HeroDraws>))
            .add_systems(
                PostUpdate,
                // After everything that moves the camera or sets the near plane (`keep_camera_above_ground`), before
                // the projection turns into this frame's matrix. Nothing here runs while the mod is off.
                draw::update_hero
                    .run_if(hero_shown)
                    .after(UniverseSet::Camera)
                    .after(crate::terrain::update_node_transforms)
                    .before(UniverseSet::FloatingOrigin)
                    .before(bevy::camera::CameraUpdateSystems),
            );
    }
}

/// The mod is on, its files read, and nothing of him made yet.
fn first_enable(control: Res<ModControl>, assets: Option<Res<HeroAssets>>, draws: Option<Res<HeroDraws>>) -> bool {
    control.hero && assets.is_some() && draws.is_none()
}

/// The mod is on and he is made.
fn hero_shown(control: Res<ModControl>, assets: Option<Res<HeroAssets>>, draws: Option<Res<HeroDraws>>) -> bool {
    control.hero && assets.is_some() && draws.is_some()
}

/// Where he is until step 4's frames: on a body (keeps his place as it moves and turns) or fixed in the
/// universe far from any.
#[derive(Clone, Copy, Debug)]
pub enum Place {
    Free(UPos),
    /// `local`: in the body's frame, from its centre (m).
    On { body: Entity, local: DVec3 },
}

/// Which light he takes (`hero light free` forces the CPU's, to compare them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LightPath {
    #[default]
    Auto,
    Free,
}

#[derive(Resource)]
pub struct Hero {
    pub place: Place,
    /// Port axes (-Z forward, +Y up) to the universe's (`Place::Free`) or the body's frame (`Place::On`).
    pub rot: DQuat,
    /// Held on the ground under him as it loads (dev command `hero stand`).
    pub stand: bool,
    /// Real seconds, whatever the game clock does (T8).
    pub clock: f64,
    pub sampler: Box<Sampler>,
    pub light_path: LightPath,
    /// Placed since the mod was turned on.
    placed: bool,
}

impl Default for Hero {
    fn default() -> Self {
        Hero {
            place: Place::Free(UPos::ZERO),
            rot: DQuat::IDENTITY,
            stand: false,
            clock: 0.0,
            sampler: Box::default(),
            light_path: LightPath::Auto,
            placed: false,
        }
    }
}

/// A rotation with -Z towards `look` and +Y as near `up` as can be.
fn facing(look: DVec3, up: DVec3) -> DQuat {
    let z = -look.normalize_or(DVec3::NEG_Z);
    let x = up.cross(z).try_normalize().unwrap_or_else(|| z.any_orthonormal_vector());
    DQuat::from_mat3(&DMat3::from_cols(x, z.cross(x), z))
}

type Bodies<'w, 's> = Query<'w, 's, (Entity, &'static SystemBody, Option<&'static Terrain>)>;

/// The body a point is attached to: the nearest within `ATTACH_RADII` of its radius.
fn attach_body<'a>(pos: UPos, bodies: &'a Bodies) -> Option<(Entity, &'a SystemBody, Option<&'a Terrain>)> {
    bodies
        .iter()
        .filter(|(_, b, _)| !b.star && b.radius_m > 0.0)
        .map(|x| (x, x.1.pos.delta_m(pos).length() - x.1.radius_m))
        .filter(|(x, h)| *h < x.1.radius_m * tune::ATTACH_RADII)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(x, _)| x)
}

impl Hero {
    /// F2: at the camera's spot plus forward, facing it (the camera stays free in this step), lifted if that's
    /// under the ground.
    fn place_ahead(&mut self, cam: &FlyCamera, bodies: &Bodies) {
        self.placed = true;
        self.stand = false;
        self.put(cam, bodies, tune::SPAWN_AHEAD_M);
    }

    fn put(&mut self, cam: &FlyCamera, bodies: &Bodies, ahead: f64) {
        let forward = cam.rot * DVec3::NEG_Z;
        let mut pos = cam.pos.offset_m(forward * ahead);
        if let Some((_, b, t)) = attach_body(pos, bodies) {
            let local = b.orient.inverse() * pos.delta_m(b.pos);
            let (ground, _) = crate::ship::ground_on(b.radius_m, b.shape, t, local);
            if local.length() < ground + 1.0 {
                pos = b.pos.offset_m(b.orient * (local.normalize_or(DVec3::Y) * (ground + tune::SPAWN_LIFT_M)));
            }
        }
        self.set_pose(cam, bodies, pos);
    }

    /// At `pos`, facing the camera, upright on the body he's attached to, else as the camera.
    fn set_pose(&mut self, cam: &FlyCamera, bodies: &Bodies, pos: UPos) {
        let look = cam.pos.delta_m(pos);
        match attach_body(pos, bodies) {
            Some((body, b, _)) => {
                let up = pos.delta_m(b.pos).normalize_or(DVec3::Y);
                self.rot = b.orient.inverse() * facing(look, up);
                self.place = Place::On { body, local: b.orient.inverse() * pos.delta_m(b.pos) };
            }
            None => {
                self.rot = facing(look, cam.rot * DVec3::Y);
                self.place = Place::Free(pos);
            }
        }
    }

    /// His position and rotation in the universe. If the body he was on has gone (another system), he appears
    /// in front of the camera again.
    fn world_pose(&mut self, cam: &FlyCamera, bodies: &Bodies) -> (UPos, DQuat) {
        match self.place {
            Place::Free(pos) => (pos, self.rot),
            Place::On { body, local } => match bodies.get(body) {
                Ok((_, b, _)) => (b.pos.offset_m(b.orient * local), b.orient * self.rot),
                Err(_) => {
                    self.place_ahead(cam, bodies);
                    let (pos, rot) = (cam.pos.offset_m(cam.rot * DVec3::NEG_Z * tune::SPAWN_AHEAD_M), self.rot);
                    match self.place {
                        Place::Free(p) => (p, rot),
                        Place::On { .. } => (pos, cam.rot),
                    }
                }
            },
        }
    }

    /// `hero stand`: his soles on the ground as it loads and refines.
    fn keep_standing(&mut self, assets: &HeroAssets, bodies: &Bodies) {
        if !self.stand {
            return;
        }
        let Place::On { body, local } = self.place else { return };
        let Ok((_, b, t)) = bodies.get(body) else { return };
        let (ground, level) = crate::ship::ground_on(b.radius_m, b.shape, t, local);
        if level.is_some() {
            let clip = self.sampler.layers[0].clip.and_then(|c| assets.clips.clips.get(c as usize));
            let sole = clip.map_or(-1.0, |c| c.ground_z) as f64;
            self.place = Place::On { body, local: local.normalize_or(DVec3::Y) * (ground - sole) };
        }
    }

    fn start_clip(&mut self, clips: &se_core::hero_asset::ClipSet, name: &str) {
        match clips.find(name) {
            Some(c) => self.sampler.play(c as u16, 0.0),
            None => warn!("viltrumite: no clip '{name}'"),
        }
    }
}

/// `hero here [M]` | `stand [M]` | `face DEG` | `clip NAME` | `light auto|free` | `state`: test runs put him where
/// a picture needs him. `here`: M (default 6) in front of the camera, facing it. `stand`: on the ground M (default
/// 6) ahead along the ground, facing the camera. Step 4 replaces the placing with `keys` and `aim`.
pub fn dev_command(world: &mut World, arg: &str) {
    let (verb, rest) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
    let number = |d: f64| rest.parse::<f64>().unwrap_or(d);
    let verb = verb.to_ascii_lowercase();
    match verb.as_str() {
        "here" | "stand" => {
            let ahead = number(tune::SPAWN_AHEAD_M);
            let stand = verb == "stand";
            let _ = world.run_system_cached_with(place_hero, (ahead, stand));
        }
        "face" => {
            let deg = number(0.0);
            let mut hero = world.resource_mut::<Hero>();
            let turn = DQuat::from_rotation_y(deg.to_radians());
            hero.rot *= turn;
        }
        "clip" => {
            let name = rest.to_string();
            world.resource_scope(|world, assets: Mut<HeroAssets>| {
                world.resource_mut::<Hero>().start_clip(&assets.clips, &name);
            });
        }
        "light" => world.resource_mut::<Hero>().light_path = if rest.eq_ignore_ascii_case("free") { LightPath::Free } else { LightPath::Auto },
        "state" => {
            let _ = world.run_system_cached(log_state);
        }
        _ => warn!("hero: unknown command '{arg}'"),
    }
}

fn place_hero(In((ahead, stand)): In<(f64, bool)>, mut hero: ResMut<Hero>, cam: Res<FlyCamera>, bodies: Bodies) {
    if !stand {
        hero.stand = false;
        hero.put(&cam, &bodies, ahead);
        return;
    }
    // Along the ground from under the camera.
    let Some((_, b, _)) = attach_body(cam.pos, &bodies) else {
        warn!("hero stand: no body near");
        return;
    };
    let up = cam.pos.delta_m(b.pos).normalize_or(DVec3::Y);
    let forward = cam.rot * DVec3::NEG_Z;
    let along = (forward - up * forward.dot(up)).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
    let pos = cam.pos.offset_m(along * ahead);
    hero.set_pose(&cam, &bodies, pos);
    hero.stand = true;
}

fn log_state(hero: Res<Hero>, control: Res<ModControl>, cam: Res<FlyCamera>, bodies: Bodies, pipes: Res<crate::pipeline_watch::GroundPipelines>) {
    let pos = match hero.place {
        Place::Free(p) => p,
        Place::On { body, local } => bodies.get(body).map_or(UPos::ZERO, |(_, b, _)| b.pos.offset_m(b.orient * local)),
    };
    info!(
        "hero: on {} loading {} pipelines {:?}, {}, {:.2} m from the camera, clock {:.2} s, clip {:?} time {:.2} s",
        control.hero,
        control.loading,
        pipes.hero(),
        draw::describe(&hero.place, &bodies),
        pos.delta_m(cam.pos).length(),
        hero.clock,
        hero.sampler.layers[0].clip,
        hero.sampler.layers[0].time,
    );
}
