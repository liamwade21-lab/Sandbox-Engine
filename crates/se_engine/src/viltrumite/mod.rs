//! The Viltrumite powers mod (docs/spec-mods-viltrumite-powers.md): a playable Omni-Man. The menu, the registry
//! and the files' checks are `crate::mods`. So far (step 3): he is drawn, lit as the ground and plays `bf_idle`,
//! hovering in front of the camera; the camera stays free. Step 4 brings flight and the real frames (`Hero`'s
//! `Place` is the simple version of them, and the dev commands' `here`, `stand` and `face` go with it).

pub mod anim;
pub mod draw;
pub mod tune;

use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use se_core::catalog::ObjectRef;
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
                    .before(bevy::camera::CameraUpdateSystems)
                    // (The capture views copy the near plane he sets, in the same frame.)
                    .before(crate::capture::follow_main),
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

/// Where he is until step 4's frames. The camera stays free in this step, so he rides what carries it
/// (`FlyCamera::follow`), whatever the distance to any body: bodies move at tens of km/s and turn at hundreds of
/// m/s, so in any other frame he leaves the view within a frame or two.
#[derive(Clone, Copy, Debug)]
pub enum Place {
    /// Fixed in the universe, as a camera bound to nothing.
    Free,
    /// With what the camera is bound to, in the frame that carries the camera (`Follow::last_rot`: it turns only
    /// while the camera turns with the body). `local`: from its centre, in that frame (m).
    Rides { target: ObjectRef, locked: bool, local: DVec3 },
    /// On a body's ground (`hero stand`). `local`: in the body's frame, from its centre (m).
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
    /// Port axes (-Z forward, +Y up) to the frame of his `place`: the universe's (`Free`), what carries the
    /// camera (`Rides`), the body's (`On`).
    pub rot: DQuat,
    /// His position and rotation in the universe, as of the last time they were worked out.
    pub world: (UPos, DQuat),
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
            place: Place::Free,
            rot: DQuat::IDENTITY,
            world: (UPos::ZERO, DQuat::IDENTITY),
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

/// The body a point counts as near: the nearest within `ATTACH_RADII` of its radius.
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
        // (A giant has no ground: the camera may be under its radius, and he with it.)
        if let Some((_, b, t)) = attach_body(pos, bodies).filter(|(_, _, t)| !t.is_some_and(|t| t.proc.kind.is_giant())) {
            let local = b.orient.inverse() * pos.delta_m(b.pos);
            let (ground, _) = crate::ship::ground_on(b.radius_m, b.shape, t, local);
            if local.length() < ground + 1.0 {
                pos = b.pos.offset_m(b.orient * (local.normalize_or(DVec3::Y) * (ground + tune::SPAWN_LIFT_M)));
            }
        }
        self.ride(cam, pos, upright(cam, bodies, pos));
    }

    /// At `pos`, turned `rot` (both the universe's), in the frame that carries the camera.
    fn ride(&mut self, cam: &FlyCamera, pos: UPos, rot: DQuat) {
        match cam.follow {
            Some(f) => {
                let to_local = f.last_rot.inverse();
                self.rot = to_local * rot;
                self.place = Place::Rides { target: f.target, locked: f.locked, local: to_local * pos.delta_m(f.last_pos) };
            }
            None => {
                self.rot = rot;
                self.place = Place::Free;
            }
        }
        self.world = (pos, rot);
    }

    /// Where riding the camera's carrier puts him now. If the camera's binding changed, he stays where he is and
    /// rides what carries it from there.
    fn carried(&mut self, cam: &FlyCamera) {
        let rides = match self.place {
            Place::Rides { target, locked, .. } => Some((target, locked)),
            _ => None,
        };
        if cam.follow.map(|f| (f.target, f.locked)) != rides {
            let (pos, rot) = self.world;
            self.ride(cam, pos, rot);
        }
        if let (Place::Rides { local, .. }, Some(f)) = (self.place, cam.follow) {
            self.world = (f.last_pos.offset_m(f.last_rot * local), f.last_rot * self.rot);
        }
    }

    /// His position and rotation in the universe, this frame. If the body he stood on has gone (another system),
    /// he appears in front of the camera again.
    fn world_pose(&mut self, cam: &FlyCamera, bodies: &Bodies) -> (UPos, DQuat) {
        match self.place {
            Place::On { body, local } => match bodies.get(body) {
                Ok((_, b, _)) => self.world = (b.pos.offset_m(b.orient * local), b.orient * self.rot),
                Err(_) => self.place_ahead(cam, bodies),
            },
            _ => self.carried(cam),
        }
        self.world
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

/// Facing the camera from `pos`, upright on the body near him, else (or looking straight along that up) as the
/// camera.
fn upright(cam: &FlyCamera, bodies: &Bodies, pos: UPos) -> DQuat {
    let look = cam.pos.delta_m(pos);
    let along = look.normalize_or_zero();
    let up = attach_body(pos, bodies).map(|(_, b, _)| pos.delta_m(b.pos).normalize_or(DVec3::Y)).filter(|up| up.dot(along).abs() < 0.95);
    facing(look, up.unwrap_or(cam.rot * DVec3::Y))
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
            // (No clips until the mod's first enable, or if its files were refused.)
            let done = world.try_resource_scope(|world, assets: Mut<HeroAssets>| {
                world.resource_mut::<Hero>().start_clip(&assets.clips, &name);
            });
            if done.is_none() {
                warn!("hero clip: the mod's files aren't loaded (turn it on first)");
            }
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
    let Some((body, b, _)) = attach_body(cam.pos, &bodies) else {
        warn!("hero stand: no body near");
        return;
    };
    let up = cam.pos.delta_m(b.pos).normalize_or(DVec3::Y);
    let forward = cam.rot * DVec3::NEG_Z;
    let along = (forward - up * forward.dot(up)).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
    let pos = cam.pos.offset_m(along * ahead);
    let rot = upright(&cam, &bodies, pos);
    hero.rot = b.orient.inverse() * rot;
    hero.place = Place::On { body, local: b.orient.inverse() * pos.delta_m(b.pos) };
    hero.world = (pos, rot);
    hero.stand = true;
}

fn log_state(hero: Res<Hero>, control: Res<ModControl>, cam: Res<FlyCamera>, bodies: Bodies, pipes: Res<crate::pipeline_watch::GroundPipelines>, draws: Option<Res<HeroDraws>>) {
    info!(
        "hero: on {} loading {} pipelines {:?}, {}, {:.2} m from the camera, clock {:.2} s, clip {:?} time {:.2} s, street lamps {:?}",
        control.hero,
        control.loading,
        pipes.hero(),
        draw::describe(&hero.place, &bodies),
        hero.world.0.delta_m(cam.pos).length(),
        hero.clock,
        hero.sampler.layers[0].clip,
        hero.sampler.layers[0].time,
        draws.map_or([0.0; 3], |d| d.town()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Follow;

    /// What `travel_and_move` does to the camera when its target has moved and turned.
    fn carry(cam: &mut FlyCamera, pos: UPos, rot: DQuat) {
        let mut f = cam.follow.unwrap();
        let (off, turn) = (f.last_rot.inverse() * cam.pos.delta_m(f.last_pos), f.last_rot.inverse() * cam.rot);
        cam.pos = pos.offset_m(rot * off);
        cam.rot = rot * turn;
        (f.last_pos, f.last_rot) = (pos, rot);
        cam.follow = Some(f);
    }

    fn follow(locked: bool) -> Follow {
        Follow { target: ObjectRef::Body(3), last_pos: UPos::ZERO.offset_m(DVec3::X * 1e11), last_rot: DQuat::from_rotation_y(0.3), locked, sync: locked }
    }

    /// He stays where the camera sees him: this was seconds' worth of km/s away before he rode what carries it.
    #[test]
    fn he_rides_what_carries_the_camera() {
        for locked in [false, true] {
            let mut f = follow(locked);
            if !locked {
                f.last_rot = DQuat::IDENTITY;
            }
            let mut cam = FlyCamera { follow: Some(f), pos: f.last_pos.offset_m(DVec3::Y * 7e6), ..default() };
            let mut hero = Hero::default();
            hero.ride(&cam, cam.pos.offset_m(DVec3::NEG_Z * 6.0), DQuat::IDENTITY);
            let seen = cam.rot.inverse() * hero.world.0.delta_m(cam.pos);
            // A second of Earth's motion and, turning with it, of its spin.
            let turn = if locked { DQuat::from_rotation_y(0.3 + 7.3e-5) } else { DQuat::IDENTITY };
            carry(&mut cam, f.last_pos.offset_m(DVec3::Z * 3e4), turn);
            hero.carried(&cam);
            let now = cam.rot.inverse() * hero.world.0.delta_m(cam.pos);
            assert!((now - seen).length() < 1e-3, "locked {locked}: {seen} then {now}");
        }
    }

    /// Bound to nothing, the camera stays and so does he.
    #[test]
    fn he_stays_with_an_unbound_camera() {
        let cam = FlyCamera { pos: UPos::ZERO.offset_m(DVec3::X * 1e11), ..default() };
        let mut hero = Hero::default();
        let at = cam.pos.offset_m(DVec3::NEG_Z * 6.0);
        hero.ride(&cam, at, DQuat::IDENTITY);
        hero.carried(&cam);
        assert!(matches!(hero.place, Place::Free));
        assert!(hero.world.0.delta_m(at).length() < 1e-3);
    }

    /// Binding or unbinding the camera leaves him where he is, in the new frame.
    #[test]
    fn a_new_binding_keeps_him_where_he_is() {
        let f = follow(true);
        let mut cam = FlyCamera { pos: f.last_pos.offset_m(DVec3::Y * 7e6), ..default() };
        let mut hero = Hero::default();
        let at = cam.pos.offset_m(DVec3::NEG_Z * 6.0);
        hero.ride(&cam, at, DQuat::IDENTITY);
        cam.follow = Some(f);
        hero.carried(&cam);
        assert!(matches!(hero.place, Place::Rides { locked: true, .. }));
        assert!(hero.world.0.delta_m(at).length() < 1e-3, "{}", hero.world.0.delta_m(at));
        cam.follow = None;
        hero.carried(&cam);
        assert!(matches!(hero.place, Place::Free));
        assert!(hero.world.0.delta_m(at).length() < 1e-3);
    }
}
