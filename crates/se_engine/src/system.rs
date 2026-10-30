//! The star system around the observer.
//!
//! SpaceEngine only fully simulates the system the camera is in. We pick the
//! nearest catalog star, spawn an entity for every star/body of its system, keep
//! their positions and orientations current from the ephemeris and decide how
//! each one is drawn: a precise point sprite when it is tiny on screen, or a
//! sphere / terrain when it is resolved.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use se_core::catalog::{BodyTag, ObjectRef};
use se_core::ephemeris::Ephemeris;
use se_core::star::StarKind;
use se_core::units::{METERS_PER_AU, METERS_PER_PC};
use se_core::upos::UPos;

use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::stars::{StarMaterial, StarSettings, SunSpriteMaterial, build_sprite_mesh, push_star, star_render_color, uniforms_for};
use crate::universe::{MeterScale, SimTime, UOrientation, UPosition, UniverseSet};

/// Marks every entity owned by the active system (despawned on system change).
#[derive(Component)]
pub struct SystemMember;

#[derive(Component, Debug, Clone)]
pub struct SystemBody {
    pub obj: ObjectRef,
    pub radius_m: f64,
    pub star: bool,
    pub pos: UPos,
    pub orient: DQuat,
    /// Projected radius in pixels this frame.
    pub px_radius: f64,
    /// SE's ellipsoid: the semi-axes in the body frame relative to `radius_m`
    /// (`BodyRecord::shape`; 1 for a sphere).
    pub shape: DVec3,
}

impl SystemBody {
    /// A body-frame point of the sphere drawn as this body's ellipsoid.
    pub fn to_shape(&self, p: DVec3) -> DVec3 {
        p * self.shape
    }

    /// A body-frame point of the ellipsoid on the sphere the terrain is built on.
    pub fn from_shape(&self, p: DVec3) -> DVec3 {
        p / self.shape
    }
}

/// How many stars SE lights a body with (MAX_LIGHTS in its planet, sky and ring shaders,
/// FUN_1404bf5c0 stops gathering at 4).
pub const MAX_LIGHTS: usize = 4;
const _: () = assert!(crate::atmosphere::SKY_EXTRA_LIGHTS == MAX_LIGHTS - 1);

/// One of the further stars lighting a body, after its brightest one.
#[derive(Debug, Clone, Copy)]
pub struct ExtraLight {
    /// Unit direction (universe/render frame) from the body towards the star.
    pub dir: Vec3,
    /// The light's colour times its intensity relative to the brightest star.
    pub color: Vec3,
    /// The same on rings (0 for a star fainter than `RingsLightingFadeEndMagn`).
    pub rings_color: Vec3,
}

impl Default for ExtraLight {
    fn default() -> Self {
        Self { dir: Vec3::Y, color: Vec3::ZERO, rings_color: Vec3::ZERO }
    }
}

/// Illumination of a body by its brightest star, plus up to [`MAX_LIGHTS`] - 1 further ones in
/// `extra` (SE lights a planet in a multiple system by each of its suns).
#[derive(Component, Debug, Clone, Copy)]
pub struct BodyLighting {
    /// Unit direction (universe/render frame) from the body towards the star.
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    /// SE's light intensity ([`LightingFade::planet_intensity`]): 1 under a star brighter than the
    /// fade magnitude wherever the body is, less as the star fades, 0 unlit.
    pub sun_intensity: f32,
    /// The intensity on rings, which lose a star fainter than `RingsLightingFadeEndMagn`.
    pub rings_intensity: f32,
    pub star_pos: UPos,
    pub star_radius_m: f64,
    /// The star's effective temperature (K) and its heating L / d^2 (solar luminosities per AU^2),
    /// for SE's thermal emission.
    pub star_teff: f32,
    pub heat: f32,
    /// The further stars, brightest first; the first `n_extra` are used.
    pub extra: [ExtraLight; MAX_LIGHTS - 1],
    pub n_extra: usize,
}

impl BodyLighting {
    /// The further lights for a shader: `dir` xyz towards the star, w 1 when used; `color` rgb
    /// the light (`rings` picks the ring intensities).
    pub fn extra_uniforms(&self, rings: bool) -> ([Vec4; MAX_LIGHTS - 1], [Vec4; MAX_LIGHTS - 1]) {
        let mut dirs = [Vec4::ZERO; MAX_LIGHTS - 1];
        let mut colors = [Vec4::ZERO; MAX_LIGHTS - 1];
        for (i, l) in self.extra.iter().take(self.n_extra).enumerate() {
            dirs[i] = l.dir.extend(1.0);
            colors[i] = if rings { l.rings_color } else { l.color }.extend(0.0);
        }
        (dirs, colors)
    }
}

impl Default for BodyLighting {
    fn default() -> Self {
        Self {
            sun_dir: Vec3::Y,
            sun_color: Vec3::ONE,
            sun_intensity: 1.0,
            rings_intensity: 1.0,
            star_pos: UPos::ZERO,
            star_radius_m: 0.0,
            star_teff: 5772.0,
            heat: 0.0,
            extra: [ExtraLight::default(); MAX_LIGHTS - 1],
            n_extra: 0,
        }
    }
}

/// Point sprite of a star of the system.
#[derive(Component)]
pub struct BodySprite {
    pub owner: Entity,
    pub material: Handle<SunSpriteMaterial>,
}

/// A planet, moon or minor body drawn as a point in the shared sprite batch.
#[derive(Component)]
pub struct BatchedSprite {
    pub owner: Entity,
    pub rgb: [f32; 3],
}

/// One mesh holding the point sprites of every non-star body, rebuilt each frame:
/// one draw and one material instead of one per body.
#[derive(Resource)]
struct BodySpriteBatch {
    mesh: Handle<Mesh>,
    material: Handle<StarMaterial>,
}

/// Emissive sphere of a star.
#[derive(Component)]
pub struct StarSphere;

/// Atmosphere sky shell belonging to a body.
#[derive(Component)]
pub struct SkyOf {
    pub body: Entity,
}

#[derive(Resource, Default)]
pub struct ActiveSystem {
    pub root: Option<u32>,
    pub name: String,
    pub bodies: Vec<Entity>,
    /// Nearest catalog star and its distance in parsecs.
    pub nearest_star: Option<(u32, f64)>,
}

/// Env `SE_FRAMETIME_LOG=path`: appends one line per frame, `frame ms nearest-star`, for hitch hunting.
fn log_frame_times(active: Res<ActiveSystem>, data: Res<SeData>, mut st: Local<(Option<std::fs::File>, Option<std::time::Instant>, u64)>) {
    use std::io::Write;
    let (file, last, n) = &mut *st;
    if file.is_none() {
        let Some(path) = std::env::var_os("SE_FRAMETIME_LOG") else { return };
        *file = std::fs::File::create(path).ok();
    }
    let now = std::time::Instant::now();
    if let (Some(f), Some(prev)) = (file.as_mut(), *last) {
        let near = active.nearest_star.map(|(i, _)| data.catalog.stars[i as usize].names[0].as_str()).unwrap_or("-");
        let _ = writeln!(f, "{} {:.2} {}", *n, (now - prev).as_secs_f64() * 1000.0, near);
    }
    *last = Some(now);
    *n += 1;
}

/// SE's fade of starlight on planets and rings (`Lighting` in config/main-def.cfg, FUN_1404bb110 and
/// FUN_1404bf5c0): a star fainter than `PlanetLightingFadeBeginMagn` as seen from a body lights it
/// less, linearly in flux, and not at all past `PlanetLightingFadeEndMagn`. Rings drop a star fainter
/// than `RingsLightingFadeEndMagn` (SE never reads `RingsLightingFadeBeginMagn`).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LightingFade {
    pub planet_begin_mag: f64,
    pub planet_end_mag: f64,
    pub rings_end_mag: f64,
    /// SE's `EnablePlanetShine` and `PlanetShineBright`: bodies lit by the light other planets and
    /// moons reflect onto them.
    pub planet_shine: bool,
    pub planet_shine_bright: f64,
}

impl Default for LightingFade {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self { planet_begin_mag: -7.0, planet_end_mag: -5.0, rings_end_mag: -10.0, planet_shine: true, planet_shine_bright: 1.0 }
    }
}

impl LightingFade {
    pub fn from_main_def(text: &str) -> Self {
        let doc = se_core::sc::parse(text);
        let num = |k: &str, d: f64| doc.find_deep(k).and_then(|i| i.f64()).unwrap_or(d);
        let d = Self::default();
        Self {
            planet_begin_mag: num("PlanetLightingFadeBeginMagn", d.planet_begin_mag),
            planet_end_mag: num("PlanetLightingFadeEndMagn", d.planet_end_mag),
            rings_end_mag: num("RingsLightingFadeEndMagn", d.rings_end_mag),
            planet_shine: doc.find_deep("EnablePlanetShine").and_then(|i| i.bool()).unwrap_or(d.planet_shine),
            planet_shine_bright: num("PlanetShineBright", d.planet_shine_bright),
        }
    }

    /// The flux (L_sun / AU^2, the Sun at 1 AU = 1) of a light of apparent magnitude `mag`
    /// (SE's FUN_1401df4c0 at 1 AU: a solar absolute magnitude of 4.83).
    fn flux(mag: f64) -> f64 {
        let au_pc = METERS_PER_AU / METERS_PER_PC;
        10f64.powf(0.4 * (se_core::units::SOLAR_ABS_MAG_V + 5.0 * au_pc.log10() - 5.0 - mag))
    }

    /// SE's intensity of the brightest star of flux `flux` on a planet: faded, then
    /// `(F / max(F_begin, F))^0.35`, the brightest star normalised to 1.
    pub fn planet_intensity(&self, flux: f64) -> f32 {
        self.planet_intensities(&[(flux, true)]).first().copied().unwrap_or(0.0)
    }

    /// The same for the lights `(flux, star)`, brightest first (FUN_1404bf5c0): the list ends at
    /// the first light past the fade end, each is faded, then `(F / max(F_begin, F_max))^0.35`
    /// with `F_max` over the stars only (planet shine never sets the scale).
    pub fn planet_intensities(&self, lights: &[(f64, bool)]) -> Vec<f32> {
        let (begin, end) = (Self::flux(self.planet_begin_mag), Self::flux(self.planet_end_mag));
        let lit: Vec<(f64, bool)> = lights
            .iter()
            .take_while(|l| l.0 >= end)
            .map(|&(flux, star)| (if flux < begin { ((flux - end) / (begin - end)).max(0.0) * flux } else { flux }, star))
            .collect();
        let max = lit.iter().filter(|l| l.1).fold(begin, |m, l| m.max(l.0));
        lit.iter().map(|l| (l.0 / max).powf(0.35) as f32).collect()
    }

    /// The flux past which a light is dropped (`PlanetLightingFadeEndMagn`).
    pub fn end_flux(&self) -> f64 {
        Self::flux(self.planet_end_mag)
    }

    pub fn rings_lit(&self, flux: f64) -> bool {
        flux >= Self::flux(self.rings_end_mag)
    }
}

/// SE's `AmbientLighting` (0 to 1 in steps of 0.05, none by default): a flat light on every
/// surface, day and night, so dark bodies can be seen. `>` / `<` (Shift+. / Shift+,) change it
/// (IncAmbient / DecAmbient in SE's key table, FUN_1405ba8f0).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct AmbientLighting(pub f32);

const AMBIENT_STEP: f32 = 0.05;

fn ambient_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut egui: bevy_egui::EguiContexts,
    mut ambient: ResMut<AmbientLighting>,
    mut msg: ResMut<crate::messages::Messages>,
    i18n: Res<crate::i18n::I18n>,
) {
    if egui.ctx_mut().is_ok_and(|c| c.egui_wants_keyboard_input()) {
        return;
    }
    let held = |a, b| keys.pressed(a) || keys.pressed(b);
    if !held(KeyCode::ShiftLeft, KeyCode::ShiftRight) || held(KeyCode::ControlLeft, KeyCode::ControlRight) || held(KeyCode::AltLeft, KeyCode::AltRight) {
        return;
    }
    let step = match (keys.just_pressed(KeyCode::Period), keys.just_pressed(KeyCode::Comma)) {
        (true, false) => AMBIENT_STEP,
        (false, true) => -AMBIENT_STEP,
        _ => return,
    };
    // SE rounds to its steps (FUN_1404c12c0).
    ambient.0 = ((ambient.0 + step) / AMBIENT_STEP).round().clamp(0.0, 1.0 / AMBIENT_STEP) * AMBIENT_STEP;
    msg.show(format!("{}: {:.2}", i18n.tr("Ambient lighting"), ambient.0));
}

fn load_lighting_fade(mut commands: Commands, cfg: Res<crate::config::EngineConfig>) {
    if let Ok(text) = std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")) {
        commands.insert_resource(LightingFade::from_main_def(&text));
    }
}

pub struct SystemPlugin;

impl Plugin for SystemPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveSystem>()
            .init_resource::<SystemLoad>()
            .init_resource::<LightingFade>()
            .init_resource::<AmbientLighting>()
            .add_systems(Update, ambient_keys)
            .add_plugins(MaterialPlugin::<StarSurfaceMaterial>::default())
            .add_systems(Startup, (setup_sprite_batch, load_lighting_fade))
            .add_systems(Last, log_frame_times)
            .add_systems(
                PostUpdate,
                (select_active_system, spawn_system_members, update_bodies, update_lighting)
                    .chain()
                    .after(crate::universe::advance_time)
                    .in_set(UniverseSet::Simulate),
            )
            .add_systems(
                PostUpdate,
                (update_projected_size, update_sprites, update_star_spheres, update_sky_shells)
                    .chain()
                    .after(UniverseSet::Camera)
                    .before(UniverseSet::FloatingOrigin),
            );
    }
}

/// Components of a multiple system lie at most this far from its barycentre (catalog `Separation`s
/// reach 0.25 pc).
const SYSTEM_REACH_PC: f64 = 0.5;

/// Distance (pc) from `cam_pc` to catalog star `i` where it really is: a component of a multiple
/// system at its orbital position, not at the barycentre its `pos_pc` holds.
fn star_distance_pc(cat: &se_core::catalog::Catalog, eph: &Ephemeris, i: u32, jd: f64, cam_pc: DVec3) -> f64 {
    let s = &cat.stars[i as usize];
    if s.parent.is_some() {
        eph.position(ObjectRef::Star(i), jd).to_parsecs().distance(cam_pc)
    } else {
        s.pos_pc.distance(cam_pc)
    }
}

/// The catalog star (not a barycentre) nearest to the camera and its distance in pc. The coarse
/// pass measures every star at `pos_pc`; stars that could still be nearer once their systems are
/// spread out are then measured at their real positions (Regor A is 2863 AU from its barycentre).
fn nearest_catalog_star(cat: &se_core::catalog::Catalog, eph: &Ephemeris, jd: f64, cam_pc: DVec3) -> Option<(u32, f64)> {
    let drawn = |i: usize| {
        let s = &cat.stars[i];
        s.has_position && !s.barycenter
    };
    let d0 = (0..cat.stars.len())
        .filter(|&i| drawn(i))
        .map(|i| cat.stars[i].pos_pc.distance_squared(cam_pc))
        .fold(f64::INFINITY, f64::min)
        .sqrt();
    if !d0.is_finite() {
        return None;
    }
    let reach2 = (d0 + SYSTEM_REACH_PC).powi(2);
    (0..cat.stars.len())
        .filter(|&i| drawn(i) && cat.stars[i].pos_pc.distance_squared(cam_pc) < reach2)
        .map(|i| (i as u32, star_distance_pc(cat, eph, i as u32, jd, cam_pc)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// A generator's output, its script parsed (with SE's life), how long that took, and the bodies
/// with life.
type Generated = (se_core::procgen::GeneratedSystem, se_core::sc::Block, std::time::Duration, usize);

/// Generate a star's planetary system (with SE's component stars when `multiples`), parse it and
/// run SE's life generator on it (FUN_1404faf40), with the system's seed (its root's).
fn generate(cat: &se_core::catalog::Catalog, star: u32, multiples: bool) -> impl FnOnce() -> Generated + Send + 'static {
    let (mut params, seed) = proc_system_input(&cat.stars[star as usize]);
    let root = cat.system_root(ObjectRef::Star(star)).unwrap_or(star);
    params.se_seed = Some(se_core::procgen::se_system_seed(&cat.stars[root as usize]));
    let stars: Vec<_> = std::iter::once(root)
        .chain(cat.system_members(&cat.stars[root as usize].names[0]).into_iter().filter_map(|m| match m {
            ObjectRef::Star(i) => Some(i),
            _ => None,
        }))
        .map(|i| cat.stars[i as usize].clone())
        .collect();
    let tables = cat.climate.clone();
    move || {
        let t = std::time::Instant::now();
        let g = if multiples { se_core::procgen::generate_system(&params, seed) } else { se_core::procgen::generate_planets(&params, seed) };
        let mut doc = se_core::sc::parse(&g.sc);
        let living = tables.map_or(0, |tab| se_core::procgen::apply_life_to_doc(&stars, tab, g.root_mass_sol, &mut doc, se_core::time::J2000));
        (g, doc, t.elapsed(), living)
    }
}

/// What `select_active_system` keeps between frames.
#[derive(Default)]
struct ActiveScan {
    /// Camera position (pc) and nearest catalog star at the last full scan, and frames since.
    last_pc: DVec3,
    last_star: Option<u32>,
    age: u32,
    /// Roots whose planetary system was generated (once per star, as SE).
    generated: std::collections::HashSet<u32>,
    /// The planetary system being generated off the main thread, one at a time (SE's
    /// asynchronous loader, `LoaderMode 2`): root, star and task; then its result until the
    /// catalog is free to take it.
    running: Option<(u32, u32, Task<Generated>)>,
    ready: Option<(u32, u32, Generated)>,
}

/// The members of the system just entered still to spawn, a few each frame (SE's loader works
/// to a time budget per frame, `MaxTimePerFrame`).
#[derive(Resource, Default)]
struct SystemLoad {
    pending: std::collections::VecDeque<ObjectRef>,
    /// The system's stars (for the light on its bodies) and the sphere mesh of its stars.
    lit: Vec<u32>,
    sphere: Option<Handle<Mesh>>,
    /// Time spent and bodies spawned so far.
    spent: std::time::Duration,
    bodies: u32,
}

/// Spawning time per frame for a system's members (the port's; SE's loader allows 10 ms for all
/// its loading).
const SPAWN_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

fn select_active_system(
    mut commands: Commands,
    mut data: ResMut<SeData>,
    nearest_proc: Res<crate::procstars::NearestProcStar>,
    cam: Res<FlyCamera>,
    sim: Res<SimTime>,
    mut active: ResMut<ActiveSystem>,
    members: Query<Entity, With<SystemMember>>,
    mut load: ResMut<SystemLoad>,
    mut scan: Local<ActiveScan>,
) {
    let cam_pc = cam.pos.to_parsecs();
    let cat = data.catalog.clone();
    // The full catalog scan only reruns once the camera has moved by a percent of the
    // distance to the nearest star (or every couple of seconds).
    let ActiveScan { last_pc, last_star, age, generated, running, ready } = &mut *scan;
    *age += 1;
    let mut nearest = {
        let eph = Ephemeris::new(&cat);
        let reuse = last_star
            .map(|i| (i, star_distance_pc(&cat, &eph, i, sim.jd, cam_pc)))
            .filter(|&(_, d)| *age < 120 && last_pc.distance(cam_pc) < d * 0.01);
        reuse.or_else(|| {
            let n = nearest_catalog_star(&cat, &eph, sim.jd, cam_pc);
            *last_pc = cam_pc;
            *last_star = n.map(|(i, _)| i);
            *age = 0;
            n
        })
    };
    active.nearest_star = nearest;
    // Release our Arc clone so make_mut below does not deep-copy the catalog.
    drop(cat);
    // A procedural star closer than any catalog star becomes a real catalog entry.
    let cat_dist = nearest.map(|(_, d)| d).unwrap_or(f64::INFINITY);
    if let Some(ps) = nearest_proc.star.as_ref() {
        if nearest_proc.dist_pc < cat_dist * 0.9 && nearest_proc.dist_pc < crate::procstars::PROMOTE_PC {
            let rec = procedural_star_record(ps, nearest_proc.pos_pc);
            let existing = match data.catalog.find(&rec.names[0]) {
                Some(ObjectRef::Star(i)) => Some(i),
                _ => None,
            };
            // Adding a star needs the catalog unshared: `make_mut` would deep-copy all of it
            // (a ~300 ms stall) while anything else holds a clone, so a new star waits for
            // a frame where nothing does. Its planets follow from a background task.
            let added = match existing {
                Some(_) => None,
                None => std::sync::Arc::get_mut(&mut data.catalog).map(|cat_mut| promote_star(cat_mut, rec)),
            };
            if let Some(idx) = existing.or(added) {
                nearest = Some((idx, nearest_proc.dist_pc));
                active.nearest_star = nearest;
            }
        }
    }
    if let Some(t) = running.as_mut().and_then(|(_, _, t)| check_ready(t)) {
        let (r, s, _) = running.take().unwrap();
        *ready = Some((r, s, t));
    }
    let Some((ni, _)) = nearest else { return };
    let root = data.catalog.system_root(ObjectRef::Star(ni)).unwrap_or(ni);
    if ready.as_ref().is_some_and(|(r, _, _)| *r != root) {
        // The camera has moved on: SE's loader drops a request it no longer needs. Freeing a
        // thousand parsed bodies takes milliseconds, so a worker does it.
        let stale = ready.take();
        AsyncComputeTaskPool::get().spawn(async move { drop(stale) }).detach();
    }
    if active.root == Some(root) {
        return;
    }
    // A procedural star's system, or the generated one SE gives a catalog star without catalog
    // planets unless `NoPlanets` (FUN_140510f40), once per star. It is made in the background while
    // the camera stays in its current system: at hundreds of parsecs a second the nearest star
    // changes every frame, and only a star still nearest when its system is ready is entered.
    let want = if generated.contains(&root) {
        None
    } else if needs_system(&data.catalog, root) {
        Some((root, true))
    } else {
        generated_system_star(&data.catalog, root).map(|s| (s, false))
    };
    if let Some((star, multiples)) = want {
        let Some((_, _, out)) = ready.take() else {
            if running.is_none() {
                let job = generate(&data.catalog, star, multiples);
                *running = Some((root, star, AsyncComputeTaskPool::get().spawn(async move { job() })));
            }
            return;
        };
        match std::sync::Arc::get_mut(&mut data.catalog) {
            Some(cat_mut) => {
                generated.insert(root);
                // Unless it was made meanwhile (a selection materializes a star at once).
                if !multiples || needs_system(cat_mut, root) {
                    load_generated(cat_mut, star, out);
                }
            }
            None => {
                *ready = Some((root, star, out));
                return;
            }
        }
    }
    let cat = &data.catalog;
    // Switch systems: the old members go now, the new ones spawn from the next system on, stars
    // and planets first.
    for e in &members {
        commands.entity(e).despawn();
    }
    active.bodies.clear();
    active.root = Some(root);
    active.name = cat.stars[root as usize].names[0].clone();
    let mut objs = cat.system_members(&active.name);
    objs.push(ObjectRef::Star(root));
    objs.sort_by_key(|o| match o {
        ObjectRef::Star(_) => 0,
        ObjectRef::Body(i) => match cat.bodies[*i as usize].tag {
            BodyTag::Planet | BodyTag::DwarfPlanet => 1,
            BodyTag::Moon | BodyTag::DwarfMoon => 2,
            _ => 3,
        },
        _ => 4,
    });
    *load = SystemLoad { pending: objs.into(), lit: se_core::climate::system_stars(cat, root), ..default() };
}

/// Spawn an entity for every star and body of the active system, within [`SPAWN_BUDGET`] a frame.
fn spawn_system_members(
    mut commands: Commands,
    data: Res<SeData>,
    sim: Res<SimTime>,
    mut active: ResMut<ActiveSystem>,
    mut load: ResMut<SystemLoad>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut star_mats: ResMut<Assets<SunSpriteMaterial>>,
    (mut surf_mats, mut star_noise): (ResMut<Assets<StarSurfaceMaterial>>, Local<Option<Handle<Image>>>),
    settings: Res<StarSettings>,
    (mut images, mut atm_cache): (ResMut<Assets<Image>>, ResMut<crate::atmosphere::AtmosphereCache>),
    mut ring_mats: ResMut<Assets<crate::rings::RingMaterial>>,
    defaults: Option<Res<crate::terrain::DefaultTextures>>,
    mut sky_mats: ResMut<Assets<crate::atmosphere::SkyMaterial>>,
) {
    atm_cache.start_preload(data.vfs.clone());
    if load.pending.is_empty() {
        return;
    }
    let cat = &data.catalog;
    let eph = Ephemeris::new(cat);
    let sphere = load.sphere.get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().uv(256, 128))).clone();
    let lit = std::mem::take(&mut load.lit);
    let mut n_bodies = 0;
    let t = std::time::Instant::now();
    while let Some(obj) = load.pending.pop_front() {
        if t.elapsed() >= SPAWN_BUDGET {
            load.pending.push_front(obj);
            break;
        }
        let radius_m = eph.radius_m(obj);
        if radius_m <= 0.0 {
            continue;
        }
        let pos = eph.position(obj, sim.jd);
        let orient = eph.orientation(obj, sim.jd);
        let star = matches!(obj, ObjectRef::Star(_));
        let shape = match obj {
            ObjectRef::Body(i) => DVec3::from_array(cat.bodies[i as usize].shape()),
            ObjectRef::Star(i) => DVec3::from_array(cat.stars[i as usize].shape()),
            _ => DVec3::ONE,
        };
        let mut ec = commands.spawn((
            SystemMember,
            SystemBody {
                obj,
                radius_m,
                star,
                pos,
                orient,
                px_radius: 0.0,
                shape,
            },
            BodyLighting::default(),
            UPosition(pos),
            UOrientation(orient),
            MeterScale(DVec3::splat(radius_m) * shape),
            Transform::IDENTITY,
            Visibility::Hidden,
        ));
        let body_entity = ec.id();
        if let ObjectRef::Star(i) = obj {
            let s = &cat.stars[i as usize];
            let rgb = star_render_color(s.class.kind, s.teff);
            if s.class.kind != StarKind::BlackHole {
                // SE's photosphere parameters (its surface generator's star branch).
                let surf = se_core::surface_gen::generate(
                    &se_core::surface_gen::star_input(s),
                    &se_core::surface_gen::given_from(s.block.sub("Surface"), None),
                );
                let noise = star_noise.get_or_insert_with(|| images.add(star_noise_image())).clone();
                ec.insert((
                    Mesh3d(sphere.clone()),
                    MeshMaterial3d(surf_mats.add(StarSurfaceMaterial {
                        u: StarSurfaceUniform::new(s.teff.max(500.0) as f32, &surf),
                        noise,
                    })),
                    StarSphere,
                ));
            }
            spawn_sprite(&mut commands, &mut meshes, &mut star_mats, &settings, body_entity, rgb, s.abs_mag as f32);
        } else if let ObjectRef::Body(i) = obj {
            let b = &cat.bodies[i as usize];
            let light = se_core::climate::star_light_from(&eph, cat, obj, sim.jd, &lit);
            let mut temperature = se_core::climate::equilibrium_temperature(light.flux_mean, b);
            // SE fills in the atmosphere and ocean of solid planets the catalog leaves without.
            let completion = cat.climate.as_ref().and_then(|t| {
                let body_in = se_core::climate::catalog_body_in(cat, i as usize, &light)?;
                let c = se_core::climate::complete(t, &body_in)?;
                debug!(
                    "climate of {}: {:.3} atm {:.0} K, ocean {:.2} km, age {:.2} Gyr, gases {:?}",
                    b.names[0],
                    c.pressure_atm,
                    c.t_surf,
                    c.ocean_depth_km,
                    light.age_gyr,
                    c.gas_percent(t).iter().take(4).collect::<Vec<_>>()
                );
                Some(c)
            });
            if let Some(c) = &completion {
                temperature = c.t_surf as f64;
            }
            // SE's surface parameters (and colour, SurfStyle) for the body.
            let greenhouse = b.block.sub("Atmosphere").and_then(|a| a.f64("Greenhouse")).unwrap_or(0.0);
            let (gen_in, given) = se_core::surface_gen::catalog_input(cat, i as usize, temperature - greenhouse, greenhouse, light.flux_mean, completion.as_ref());
            let surf_gen = se_core::surface_gen::generate(&gen_in, &given);
            // Only reasonably large bodies get a sprite; tiny ones are invisible at a distance.
            if radius_m >= 20_000.0 || matches!(b.tag, BodyTag::Planet | BodyTag::DwarfPlanet | BodyTag::Moon) {
                let c = b.color.map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).or(surf_gen.color).unwrap_or([1.0, 0.97, 0.92]);
                commands.spawn((SystemMember, BatchedSprite { owner: body_entity, rgb: c }));
            }
            let bump_km = se_core::surface_gen::catalog_bump_km(cat, i as usize, completion.as_ref());
            let arid = surf_gen.classes.volq == 1;
            let life = b.block.sub("Life").is_some();
            let mut terrain = crate::terrain::Terrain::new(radius_m, b, temperature, surf_gen, completion.as_ref(), bump_km);
            terrain.tidal_lock = se_core::climate::catalog_body_in(cat, i as usize, &light).is_some_and(|c| c.tidally_locked);
            let extras = se_core::body_params::BodyExtras::from_body_block(&b.block);
            // The catalog's atmosphere, or the look SE gives a generated one.
            // Look: Opacity, Hue, Saturation, SkyLight (`AtmosphereParamsUniform::look`).
            // Its visual height: the catalog's `Height`, else SE's (FUN_140448ef0) from a scale height in
            // air (molar mass 29; a giant's hydrogen, 2.3, at 15.6 scale heights).
            let giant = terrain.proc.kind.is_giant();
            let catalog_height = |atm: &se_core::body_params::AtmosphereParams| {
                if atm.height_km > 0.0 {
                    return atm.height_km;
                }
                let (r_km, m_earth) = (radius_m / 1000.0, b.mass_kg / se_core::units::EARTH_MASS_KG);
                let g = se_core::units::G * b.mass_kg / (radius_m * radius_m);
                if g <= 0.0 {
                    return r_km * 0.01;
                }
                if giant {
                    (8.314 * temperature / (0.0023 * g) / 1000.0 * 15.6).max(100.0).min(r_km * 0.05)
                } else {
                    let scale_h = 8.314 * (temperature + atm.greenhouse_k) / (0.029 * g) / 1000.0;
                    se_core::climate::se_visual_atm_height_km(atm.pressure_atm as f32, scale_h as f32, m_earth as f32, r_km as f32) as f64
                }
            };
            let sky = match extras.atmosphere.as_ref().filter(|_| !extras.no_atmosphere) {
                Some(atm) => Some((
                    atm.model.clone().unwrap_or_else(|| "Earth".to_string()),
                    catalog_height(atm),
                    atm.bright as f32,
                    Vec4::new(atm.opacity as f32, atm.hue.unwrap_or(0.0) as f32, atm.saturation.unwrap_or(1.0) as f32, atm.sky_light as f32),
                )),
                None => completion.as_ref().zip(cat.climate.as_ref()).and_then(|(c, t)| {
                    let look = se_core::climate::atmosphere_look(gen_in.seeds.0, c, arid, life)?;
                    Some((
                        t.atm_models[look.model].clone(),
                        (c.atm_height_km * look.height_scale) as f64,
                        look.bright,
                        Vec4::new(look.opacity, 0.0, 1.0, look.sky_light),
                    ))
                }),
            };
            if let Some((model, height_km, bright, look)) = sky {
                if let Some(gpu) = atm_cache.get_or_load(&data.vfs, &model, &mut images) {
                    let gpu = gpu.with_height(radius_m, height_km * 1000.0);
                    let bright = bright.max(0.1);
                    let shell = crate::atmosphere::spawn_sky_shell(
                        &mut commands,
                        &mut meshes,
                        &mut sky_mats,
                        &gpu,
                        radius_m,
                        height_km * 1000.0,
                        bright,
                        look,
                    );
                    commands.entity(shell).insert((
                        SystemMember,
                        SkyOf { body: body_entity },
                        UPosition(pos),
                        // Turned with the body and flattened like it (`sky.wesl` works in the shell's frame).
                        UOrientation(orient),
                        MeterScale(DVec3::splat(gpu.top_radius_m(radius_m)) * shape),
                        Visibility::Hidden,
                    ));
                    terrain.atmosphere = Some(gpu);
                    terrain.atm_bright = bright;
                    terrain.atm_look = look;
                } else {
                    warn!("atmosphere model {model} not found");
                }
            }
            match b.block.str("Model") {
                Some(model_ref) => {
                    commands.entity(body_entity).insert(crate::body_models::BodyModel::new(model_ref));
                }
                None => {
                    commands.entity(body_entity).insert(terrain);
                }
            }
            if let Some(d) = defaults.as_ref() {
                crate::rings::spawn_rings(
                    &mut commands,
                    &data.vfs,
                    &mut meshes,
                    &mut images,
                    &mut ring_mats,
                    body_entity,
                    &b.block,
                    radius_m,
                    d.white.clone(),
                );
            }
            n_bodies += 1;
        }
        active.bodies.push(body_entity);
    }
    load.lit = lit;
    load.spent += t.elapsed();
    load.bodies += n_bodies;
    if load.pending.is_empty() {
        info!("entered system {} ({} bodies) in {:.0} ms", active.name, load.bodies, load.spent.as_secs_f64() * 1e3);
    }
}

/// Add a procedural star to the catalog without its planets (a barycentre when SE makes it a
/// multiple system, its components coming with the planets); returns its star index.
pub fn promote_star(cat: &mut se_core::catalog::Catalog, mut rec: se_core::catalog::StarRecord) -> u32 {
    // SE turns a procedural star into a multiple system's barycentre (CreateProcedural).
    let (params, seed) = proc_system_input(&rec);
    rec.barycenter = se_core::procgen::is_multiple(&params, seed);
    cat.add_star_record(rec)
}

/// Add a procedural star to the catalog together with its generated planetary system, at once;
/// returns its star index.
pub fn materialize_star(cat: &mut se_core::catalog::Catalog, rec: se_core::catalog::StarRecord) -> u32 {
    let idx = promote_star(cat, rec);
    if needs_system(cat, idx) {
        let out = generate(cat, idx, true)();
        load_generated(cat, idx, out);
    }
    idx
}

/// A procedural star still without its generated system (component stars and planets).
fn needs_system(cat: &se_core::catalog::Catalog, idx: u32) -> bool {
    let s = &cat.stars[idx as usize];
    s.source == "procedural" && cat.children_of(&s.names[0]).is_empty()
}

/// Load a star's generated planetary system (with SE's component stars for a procedural star)
/// into the catalog.
fn load_generated(cat: &mut se_core::catalog::Catalog, idx: u32, (g, doc, took, living): Generated) {
    let t = std::time::Instant::now();
    cat.load_doc(doc, se_core::climate::PROCEDURAL_SOURCE);
    if let Some(m) = g.root_mass_sol {
        cat.stars[idx as usize].mass_kg = m * se_core::units::SOLAR_MASS_KG;
    }
    cat.finalize();
    let name = &cat.stars[idx as usize].names[0];
    info!(
        "generated system for {name} in {took:.0?}, loaded in {:.0?} ({} stars, {} bodies, {} binary planets, {living} with life)",
        t.elapsed(),
        g.stars.len().max(1),
        g.bodies.len(),
        g.barycenters.len()
    );
}

/// The star of a catalog system SE fills with a generated one: a system without catalog bodies
/// whose one real star (alone, or under its barycentre) has no `NoPlanets true`.
fn generated_system_star(cat: &se_core::catalog::Catalog, root: u32) -> Option<u32> {
    let r = &cat.stars[root as usize];
    if r.source == "procedural" {
        return None;
    }
    let mut stars = vec![root];
    for m in cat.system_members(&r.names[0]) {
        match m {
            ObjectRef::Body(_) => return None,
            ObjectRef::Star(i) if !stars.contains(&i) => stars.push(i),
            _ => {}
        }
    }
    let mut real = stars.into_iter().filter(|&i| !cat.stars[i as usize].barycenter);
    let star = real.next()?;
    if real.next().is_some() || cat.stars[star as usize].block.bool("NoPlanets") == Some(true) {
        return None;
    }
    Some(star)
}

/// The generator's input for a procedural star's planetary system (its star and seed).
pub fn proc_system_input(st: &se_core::catalog::StarRecord) -> (se_core::procgen::StarParams, u64) {
    let lum_bol = se_core::star::lum_from_abs_mag(st.abs_mag) * 10f64.powf(-0.4 * se_core::star::bolometric_correction(st.teff));
    // Catalog stars may lack a mass or radius: the main-sequence values for their light.
    let mass_sol = if st.mass_kg > 0.0 { st.mass_kg / se_core::units::SOLAR_MASS_KG } else { lum_bol.max(1e-4).powf(1.0 / 3.5).clamp(0.08, 80.0) };
    let radius_sol = if st.radius_km > 0.0 {
        st.radius_km / se_core::units::SOLAR_RADIUS_KM
    } else {
        se_core::star::radius_from_lum_teff(lum_bol, st.teff.max(1000.0))
    };
    // The star's own age, [Fe/H] and C/O when its record gives them (procedural stars carry SE's
    // draws, `procedural_star_record`), and SE's position seed (06-015).
    let given = |k: &str| st.block.f64(k).filter(|v| v.is_finite());
    let se_seed = se_core::procgen::se_system_seed(st);
    let params = se_core::procgen::StarParams {
        name: st.names[0].clone(),
        mass_sol,
        lum_bol,
        teff: st.teff,
        radius_sol,
        age_gyr: given("Age").unwrap_or(4.0),
        feh: given("FeH").unwrap_or(0.0),
        kind: st.class.kind,
        class: Some(st.class.clone()),
        c_to_o: given("CtoO"),
        se_seed: Some(se_seed),
    };
    let seed = se_core::proc_surface::name_seed(&params.name);
    (params, seed)
}

pub fn procedural_star_record(s: &crate::procstars::ProcStarInfo, pos_pc: DVec3) -> se_core::catalog::StarRecord {
    use se_core::star;
    let class = star::SpectralClass::parse(&s.class);
    let teff = s.teff as f64;
    let abs_mag = s.abs_mag as f64;
    let lum_bol = star::lum_from_abs_mag(abs_mag) * 10f64.powf(-0.4 * star::bolometric_correction(teff));
    // SE's star physics (procgen::se_stars), drawn on a stream of the star's name (assumed):
    // - mass from light (FUN_1401dd9f0; a giant's from its giant light, as FUN_1401d63b0 sets it
    //   after FUN_1401d70c0), and a giant's age at the end of its lifetime (FUN_1401d63b0:194-200);
    // - solitary white dwarfs, neutron stars and black holes alike: SE's mass laws, radii for the
    //   mass and the remnant draws (FUN_1401d63b0:46-56, FUN_1401d5fe0);
    // - [Fe/H] and C/O by the star's place in its galaxy (FUN_1401d63b0); cluster members (`RSC`)
    //   draw a cluster's (FUN_1401d6c60), their age and [Fe/H] coming from `clusters::member_record`.
    use se_core::procgen::se_stars as ss;
    let h = se_core::proc_surface::name_seed(&s.name);
    let mut r = se_core::surface_gen::SeRng::new(h as u32, (h >> 32) as u32);
    let (lum_code, letter, _) = se_core::climate::star_class_bits(&class);
    let remnant = ss::remnant_mass(&mut r, class.kind);
    let (mass_sol, radius_km) = match remnant {
        Some(m) => (m, ss::remnant_radius_km(class.kind, m).unwrap_or(0.0)),
        None => (ss::mass_from_lum(lum_bol, lum_code, letter), star::radius_from_lum_teff(lum_bol, teff) * se_core::units::SOLAR_RADIUS_KM),
    };
    let lifetime = se_core::climate::star_lifetime_gyr(mass_sol as f32, lum_code) as f64;
    let draw = if remnant.is_some() {
        ss::remnant_star(&mut r)
    } else {
        let x = crate::procstars::galaxy_radius_fraction(pos_pc).unwrap_or(2.0 / 3.0);
        let mut d = ss::field_star(&mut r, x, lifetime);
        if s.name.starts_with("RSC ") {
            d = ss::group_star(&mut r, d.age_gyr);
        }
        if lum_code <= 6 {
            d.age_gyr = lifetime.min(ss::MAX_AGE_GYR);
        }
        d
    };
    let block = se_core::sc::parse(&format!("Age {}\nFeH {}\nCtoO {}\n", draw.age_gyr, draw.feh, draw.c_to_o));
    se_core::catalog::StarRecord {
        names: vec![s.name.clone()],
        barycenter: false,
        parent: None,
        pos_pc,
        has_position: true,
        class,
        abs_mag,
        teff,
        radius_km,
        mass_kg: mass_sol * se_core::units::SOLAR_MASS_KG,
        orbit: None,
        rotation: se_core::rotation::RotationModel::default(),
        source: "procedural".into(),
        block,
    }
}

fn spawn_sprite(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<SunSpriteMaterial>,
    settings: &StarSettings,
    owner: Entity,
    rgb: [f32; 3],
    abs_mag: f32,
) {
    let (mut p, mut d, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    push_star(&mut p, &mut d, &mut c, &mut i, [0.0; 3], rgb, abs_mag);
    let mesh = meshes.add(build_sprite_mesh(p, d, c, i));
    let material = mats.add(SunSpriteMaterial {
        uniforms: uniforms_for(Vec3::ZERO, settings),
    });
    commands.spawn((
        SystemMember,
        Mesh3d(mesh),
        MeshMaterial3d(material.clone()),
        Transform::IDENTITY,
        Visibility::Visible,
        NoFrustumCulling,
        BodySprite { owner, material },
    ));
    let _ = RenderAssetUsages::RENDER_WORLD;
}

fn update_bodies(
    data: Res<SeData>,
    sim: Res<SimTime>,
    mut q: Query<(&mut SystemBody, &mut UPosition, &mut UOrientation)>,
) {
    let eph = Ephemeris::new(&data.catalog);
    // Thousands of bodies (asteroids, comets) in the Solar System: spread over threads.
    q.par_iter_mut().for_each(|(mut b, mut pos, mut rot)| {
        b.pos = eph.position(b.obj, sim.jd);
        b.orient = eph.orientation(b.obj, sim.jd);
        pos.0 = b.pos;
        rot.0 = b.orient;
    });
}

/// SE's colour of a star's light on its planets (FUN_1404d6970): the star's colour, brightest channel
/// 1, pulled towards its luma by the illumination saturation: `SunIllumSaturation`, or with
/// `IllumSaturationAuto` [`auto_illum_saturation`] of the star's temperature.
pub(crate) fn illum_color(c: Vec3, teff: f32, s: &StarSettings) -> Vec3 {
    let c = c / c.max_element().max(1e-3);
    let sat = if s.illum_saturation_auto { auto_illum_saturation(teff) } else { s.illum_saturation };
    let luma = Vec3::splat((0.299 * c.x * c.x + 0.587 * c.y * c.y + 0.114 * c.z * c.z).sqrt());
    luma + (c - luma) * sat
}

/// SE's automatic illumination saturation for a star of temperature `t` (FUN_1401a5270, its default
/// colour mode 2), approximating how the eye sees the light: 0.87 at 2000 K, none near 6000 K,
/// 0.45 at 10000 K and 0.75 from 18700 K.
fn auto_illum_saturation(t: f32) -> f32 {
    if t <= 6000.0 {
        (1.071_384_8 - t * 5.220_866_3e-5 - t * t * 2.335_497_3e-8).clamp(0.0, 1.0)
    } else if t <= 6700.0 {
        t * 2.08 / 6000.0 - 2.08
    } else if t < 18700.0 {
        ((t - 5200.0) / 24000.0).sqrt()
    } else {
        0.75
    }
}

/// A light on a body: flux (L_sun / AU^2), direction towards it, colour, and whether it is a star
/// (else planet shine).
struct Light {
    flux: f64,
    dir: Vec3,
    color: Vec3,
    star: bool,
}

/// SE's planet shine from body `p` onto `target` (FUN_1404da600): each star lighting `p`, of flux
/// `F` there, adds `F (R/d)^2 albedo Lambert(a) clamp(pi - asin(R/d) - a, 0, 1)` times the opposition
/// surge, `a` the phase angle at `p`; the total is scaled by the luminance of `p`'s colour (its
/// PlanetShineMode 1 colour) and the light's colour is `p`'s times the stars', brightest channel 1.
fn planet_shine(pos: UPos, radius_m: f64, albedo: f64, color: Vec3, hapke: Option<(f32, f32, f32)>, stars: &[(f64, UPos, Vec3)], target: UPos) -> Option<(f64, Vec3)> {
    use std::f64::consts::PI;
    let to_target = target.delta_m(pos);
    let d = to_target.length();
    if d <= radius_m || radius_m <= 0.0 {
        return None;
    }
    let (rd, to_target) = (radius_m / d, to_target / d);
    let max_phase = PI - rd.asin();
    let (mut total, mut rgb) = (0.0, Vec3::ZERO);
    for &(flux, star_pos, star_rgb) in stars {
        let cos = star_pos.delta_m(pos).normalize_or(DVec3::Y).dot(to_target).clamp(-1.0, 1.0);
        let a = cos.acos();
        let lambert = a.sin() / PI + (1.0 - a / PI) * cos;
        let mut f = flux * rd * rd * albedo * lambert * (max_phase - a).clamp(0.0, 1.0);
        if let Some((h, spot, inv_w)) = hapke {
            let surge = spot / (((1.0 - cos) / (1.0 + cos).max(1e-9)).sqrt() as f32 * inv_w + 1.0) + 1.0;
            f *= if h < 0.0 { 1.0 } else if h <= 1.0 { ((surge - 1.0) * h + 1.0) as f64 } else { surge as f64 };
        }
        if f > 0.0 {
            total += f;
            rgb += star_rgb * f as f32;
        }
    }
    let rgb = rgb * color;
    let max = rgb.max_element();
    (total > 0.0 && max > 0.0).then(|| (total * color.dot(Vec3::new(0.299, 0.587, 0.114)) as f64, rgb / max))
}

/// The bodies' family tree for planet shine, rebuilt when the set of bodies changes.
#[derive(Default)]
struct ShineTree {
    key: (usize, u64),
    /// The bodies (not stars) and their parents.
    entities: Vec<Entity>,
    parent: Vec<Option<ObjectRef>>,
    index: std::collections::HashMap<Entity, usize>,
    by_ref: std::collections::HashMap<ObjectRef, usize>,
    children: std::collections::HashMap<ObjectRef, Vec<usize>>,
}

impl ShineTree {
    /// SE's planet-shine sources for body `i` (FUN_1404bbf40): its own moons, then up its parents,
    /// a planet itself, and at the first star or barycentre its other children (may repeat one).
    fn sources(&self, i: usize, obj: ObjectRef, out: &mut Vec<usize>) {
        out.clear();
        out.extend(self.children.get(&obj).into_iter().flatten());
        let mut up = self.parent[i];
        for _ in 0..8 {
            let Some(r) = up else { break };
            match self.by_ref.get(&r) {
                Some(&p) => {
                    out.push(p);
                    up = self.parent[p];
                }
                None => {
                    out.extend(self.children.get(&r).into_iter().flatten().filter(|&&c| c != i));
                    break;
                }
            }
        }
    }
}

fn update_lighting(
    data: Res<SeData>,
    fade: Res<LightingFade>,
    settings: Res<StarSettings>,
    sprites: Query<&BatchedSprite>,
    terrains: Query<&crate::terrain::Terrain>,
    mut tree: Local<ShineTree>,
    mut q: Query<(Entity, &SystemBody, &mut BodyLighting)>,
) {
    let cat = &data.catalog;
    // Collect luminous stars of the system.
    let stars: Vec<(UPos, f64, Vec3, f64, f64)> = q
        .iter()
        .filter_map(|(_, b, _)| match b.obj {
            ObjectRef::Star(i) => {
                let s = &cat.stars[i as usize];
                let lum = se_core::star::lum_from_abs_mag(s.abs_mag);
                let rgb = star_render_color(s.class.kind, s.teff);
                Some((b.pos, lum, Vec3::from_array(rgb), b.radius_m, s.teff))
            }
            _ => None,
        })
        .collect();
    let flux_at = |s: &(UPos, f64, Vec3, f64, f64), pos: UPos| s.1 / (s.0.delta_m(pos).length().max(1.0) / METERS_PER_AU).powi(2);
    let brightest_at = |pos: UPos| stars.iter().enumerate().map(|(k, s)| (flux_at(s, pos), k)).max_by(|x, y| x.0.total_cmp(&y.0));
    // A body's stars of the most flux, brightest first (flux, index in `stars`).
    let star_lights = |pos: UPos| {
        let mut lit: Vec<(f64, usize)> = stars.iter().enumerate().map(|(i, s)| (flux_at(s, pos), i)).collect();
        lit.sort_by(|a, b| b.0.total_cmp(&a.0));
        lit.truncate(MAX_LIGHTS);
        lit
    };
    // The eye adapts to its sun: SE shows a red dwarf's planet tan, not deep red (the star itself is
    // still drawn in its full colour).
    let star_color = |k: usize| illum_color(stars[k].2, stars[k].4 as f32, &settings);
    let star_light = |pos: UPos, (flux, k): (f64, usize)| Light {
        flux,
        dir: stars[k].0.delta_m(pos).normalize_or(DVec3::Y).as_vec3(),
        color: star_color(k),
        star: true,
    };

    // SE lights a body with its up to MAX_LIGHTS lights of the most flux (FUN_1404bf5c0): its stars
    // and, with planet shine, the planets and moons around it. Only bodies drawn as discs (or their
    // rings) show more than their brightest star.
    let mut full: std::collections::HashMap<Entity, Vec<Light>> = std::collections::HashMap::new();
    if fade.planet_shine && q.iter().any(|(_, b, _)| !b.star && b.px_radius >= 0.5) {
        let key = q.iter().fold((0usize, 0u64), |(n, h), (e, _, _)| (n + 1, h ^ e.to_bits().wrapping_mul(0x9e37_79b9_7f4a_7c15)));
        if tree.key != key {
            *tree = ShineTree { key, ..default() };
            for (e, b, _) in &q {
                let ObjectRef::Body(i) = b.obj else { continue };
                let parent = cat.find(&cat.bodies[i as usize].parent);
                let k = tree.entities.len();
                tree.entities.push(e);
                tree.parent.push(parent);
                tree.index.insert(e, k);
                tree.by_ref.insert(b.obj, k);
                if let Some(p) = parent {
                    tree.children.entry(p).or_default().push(k);
                }
            }
        }
        // Each body's position and the most light it can reflect, `F R^2 albedo` doubled for the
        // opposition surge and times its stars (SE's albedo for a body without one: FUN_1404da600).
        let reach: Vec<(UPos, f64, f64)> = tree
            .entities
            .iter()
            .map(|&e| {
                let Ok((_, b, _)) = q.get(e) else { return (UPos::ZERO, 0.0, 0.0) };
                let ObjectRef::Body(i) = b.obj else { return (b.pos, 0.0, 0.0) };
                let albedo = Some(cat.bodies[i as usize].albedo).filter(|&a| a > 0.0).unwrap_or(0.36);
                // A body inside its star (an orbit the port cannot place) reflects nothing.
                let flux = brightest_at(b.pos).filter(|&(_, k)| stars[k].0.delta_m(b.pos).length() > stars[k].3).map_or(0.0, |l| l.0);
                (b.pos, albedo, 2.0 * flux * b.radius_m * b.radius_m * albedo * stars.len().min(MAX_LIGHTS) as f64)
            })
            .collect();
        let end_flux = fade.end_flux() / fade.planet_shine_bright.max(1e-30);
        let mut colors: Option<std::collections::HashMap<Entity, Vec3>> = None;
        let mut sources = Vec::new();
        for (e, b, _) in q.iter().filter(|(_, b, _)| !b.star && b.px_radius >= 0.5) {
            let Some(&i) = tree.index.get(&e) else { continue };
            let mut lights: Vec<Light> = star_lights(b.pos).into_iter().map(|l| star_light(b.pos, l)).collect();
            tree.sources(i, b.obj, &mut sources);
            let mut seen: Vec<usize> = Vec::new();
            for &k in &sources {
                // Most of a star's other planets are far too faint: skip them before the trigonometry.
                let (pos, albedo, power) = reach[k];
                if power < end_flux * pos.delta_m(b.pos).length_squared() || seen.contains(&k) {
                    continue;
                }
                seen.push(k);
                let src = tree.entities[k];
                let Ok((_, p, _)) = q.get(src) else { continue };
                let lit: Vec<(f64, UPos, Vec3)> = star_lights(p.pos).into_iter().map(|(f, s)| (f, stars[s].0, star_color(s))).collect();
                let colors = colors.get_or_insert_with(|| sprites.iter().map(|s| (s.owner, Vec3::from_array(s.rgb))).collect());
                let color = colors.get(&src).copied().unwrap_or(Vec3::ONE);
                let hapke = terrains.get(src).ok().map(|t| (t.surf_modulate.w, t.surf_hapke.x, t.surf_hapke.y));
                if let Some((flux, color)) = planet_shine(p.pos, p.radius_m, albedo, color, hapke, &lit, b.pos) {
                    let dir = p.pos.delta_m(b.pos).normalize_or(DVec3::Y).as_vec3();
                    lights.push(Light { flux: flux * fade.planet_shine_bright, dir, color, star: false });
                }
            }
            full.insert(e, lights);
        }
    }

    for (e, b, mut light) in &mut q {
        if b.star {
            continue;
        }
        let Some((main_flux, main_star)) = brightest_at(b.pos) else {
            // No star: SE lights nothing.
            (light.sun_intensity, light.rings_intensity, light.heat, light.n_extra) = (0.0, 0.0, 0.0, 0);
            continue;
        };
        let s = &stars[main_star];
        light.sun_dir = s.0.delta_m(b.pos).normalize_or(DVec3::Y).as_vec3();
        light.sun_color = star_color(main_star);
        light.star_pos = s.0;
        light.star_radius_m = s.3;
        light.star_teff = s.4.max(500.0) as f32;
        light.heat = main_flux as f32;
        light.n_extra = 0;
        let more = || (stars.len() > 1 && b.px_radius >= 0.15).then(|| star_lights(b.pos).into_iter().map(|l| star_light(b.pos, l)).collect());
        let Some(mut lights) = full.remove(&e).or_else(more) else {
            // Too small to show more than its brightest star (and the only one in most systems).
            light.sun_intensity = fade.planet_intensity(main_flux);
            light.rings_intensity = if fade.rings_lit(main_flux) { light.sun_intensity } else { 0.0 };
            continue;
        };
        // The brightest star stays the main light; the rest by flux.
        lights[1..].sort_by(|a, b| b.flux.total_cmp(&a.flux));
        lights.truncate(MAX_LIGHTS);
        let intensities = fade.planet_intensities(&lights.iter().map(|l| (l.flux, l.star)).collect::<Vec<_>>());
        light.sun_intensity = intensities.first().copied().unwrap_or(0.0);
        light.rings_intensity = if fade.rings_lit(main_flux) { light.sun_intensity } else { 0.0 };
        for (l, &k) in lights.iter().zip(&intensities).skip(1) {
            let color = l.color * k;
            let n = light.n_extra;
            // Rings take the stars only (SE adds just their host planet's light, not ported).
            light.extra[n] = ExtraLight { dir: l.dir, color, rings_color: if l.star && fade.rings_lit(l.flux) { color } else { Vec3::ZERO } };
            light.n_extra += 1;
        }
    }
}

/// Approximate projected radius in pixels for each body.
fn update_projected_size(
    cam: Res<FlyCamera>,
    windows: Query<&Window>,
    cams: Query<&Projection, With<MainCamera>>,
    mut q: Query<&mut SystemBody>,
) {
    let h = windows.iter().next().map(|w| w.height() as f64).unwrap_or(900.0);
    let fov = cams
        .iter()
        .next()
        .and_then(|p| match p {
            Projection::Perspective(p) => Some(p.fov as f64),
            _ => None,
        })
        .unwrap_or(1.0);
    let k = h * 0.5 / (fov * 0.5).tan();
    for mut b in &mut q {
        let d = b.pos.delta_m(cam.pos).length().max(1e-3);
        b.px_radius = if d <= b.radius_m { 1e9 } else { (b.radius_m / (d * d - b.radius_m * b.radius_m).sqrt()) * k };
    }
}

/// Apparent magnitude of a body lit by a star (Lambert phase function).
fn body_apparent_mag(data: &SeData, body: &SystemBody, light: &BodyLighting, cam: UPos) -> f64 {
    let cat = &data.catalog;
    let albedo = match body.obj {
        ObjectRef::Body(i) => cat.bodies[i as usize].albedo.clamp(0.01, 1.0),
        _ => 0.3,
    };
    let to_obs = cam.delta_m(body.pos);
    let d_obs = to_obs.length().max(1.0);
    let to_star = light.star_pos.delta_m(body.pos);
    let d_star = to_star.length().max(1.0);
    let cos_a = (to_obs / d_obs).dot(to_star / d_star).clamp(-1.0, 1.0);
    let alpha = cos_a.acos();
    let phase = ((alpha.sin() + (std::f64::consts::PI - alpha) * cos_a) / std::f64::consts::PI).max(1e-6);
    // The star's apparent magnitude at the body, from its flux there.
    let star_mag_at_body = -26.74 - 2.5 * (light.heat as f64).max(1e-30).log10();
    star_mag_at_body - 2.5 * (albedo * phase * (body.radius_m / d_obs).powi(2)).log10()
}

fn batch_uniforms(settings: &StarSettings) -> crate::stars::StarParams {
    let mut u = uniforms_for(Vec3::ZERO, settings);
    // Magnitudes arrive per vertex; planets keep their colour (SE's PLANET sprite has no greying).
    u.params2.x = 0.0;
    u.params2.y = 0.0;
    u.params2.w = 0.0;
    u.anchor = Vec4::new(0.0, 0.0, 0.0, 3.0);
    u
}

fn setup_sprite_batch(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StarMaterial>>,
    settings: Res<StarSettings>,
) {
    let (mut p, mut d, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    push_star(&mut p, &mut d, &mut c, &mut i, [0.0; 3], [0.0; 3], 99.0);
    let mesh = meshes.add(build_sprite_mesh(p, d, c, i));
    let material = mats.add(StarMaterial {
        uniforms: batch_uniforms(&settings),
    });
    commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::IDENTITY,
        Visibility::Visible,
        NoFrustumCulling,
    ));
    commands.insert_resource(BodySpriteBatch { mesh, material });
}

fn update_sprites(
    cam: Res<FlyCamera>,
    space: Res<crate::universe::ScaledSpace>,
    data: Res<SeData>,
    settings: Res<StarSettings>,
    bodies: Query<(&SystemBody, &BodyLighting)>,
    occluders: Query<(&SystemBody, Option<&crate::terrain::Terrain>)>,
    mut sprites: Query<(&BodySprite, &mut Visibility)>,
    batched: Query<&BatchedSprite>,
    batch: Option<Res<BodySpriteBatch>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StarMaterial>>,
    mut sun_mats: ResMut<Assets<SunSpriteMaterial>>,
) {
    // Stars: a few sprites with their own material.
    for (sprite, mut vis) in &mut sprites {
        let Ok((body, _)) = bodies.get(sprite.owner) else { continue };
        let rel_pc = (cam.pos.delta_m(body.pos) / METERS_PER_PC).as_vec3();
        let tint = sun_sprite_tint(body, cam.pos, &occluders);
        *vis = if tint.max_element() > 0.0 { Visibility::Visible } else { Visibility::Hidden };
        if let Some(mut m) = sun_mats.get_mut(&sprite.material) {
            m.uniforms = uniforms_for(rel_pc, &settings);
            m.uniforms.tint = tint.extend(1.0);
        }
    }
    // Everything else: one batched mesh in render space.
    let Some(batch) = batch else { return };
    if settings.is_changed() {
        if let Some(mut m) = mats.get_mut(&batch.material) {
            m.uniforms = batch_uniforms(&settings);
        }
    }
    let (mut p, mut d, mut c, mut i) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for s in &batched {
        let Ok((body, light)) = bodies.get(s.owner) else { continue };
        // Hand over to the resolved disk as it grows.
        let fade = 1.0 - ((body.px_radius - 0.5) / 1.0).clamp(0.0, 1.0) as f32;
        if fade <= 0.0 {
            continue;
        }
        let m_app = body_apparent_mag(&data, body, light, cam.pos) as f32;
        if m_app > settings.mag_limit + settings.zoom_mag() + 2.0 {
            continue;
        }
        // Anchor just in front of the body so its own disk never hides the sprite.
        // Pull in render space (after scaled-space mapping) by the body's angular
        // radius so the ratio survives the depth compression.
        let delta = body.pos.delta_m(cam.pos);
        let dist = delta.length().max(1.0);
        let (render_pos, _) = space.map(delta);
        let pull = 1.0 - (2.0 * body.radius_m / dist).min(0.5) as f32;
        let rgb = s.rgb.map(|x| x * fade);
        push_star(&mut p, &mut d, &mut c, &mut i, (render_pos * pull).to_array(), rgb, m_app);
    }
    if p.is_empty() {
        push_star(&mut p, &mut d, &mut c, &mut i, [0.0; 3], [0.0; 3], 99.0);
    }
    if let Some(mut mesh) = meshes.get_mut(&batch.mesh) {
        *mesh = build_sprite_mesh(p, d, c, i);
    }
}

/// SE's star sprite modulation (`star_sprite.glsl` MODULATE): the glare is drawn over the bodies and
/// skies, scaled by the part of the star's disc no body hides and coloured by the atmospheres the
/// line of sight crosses (a star behind a planet's limb is hidden whole, not cut by the planet).
pub(crate) fn sun_sprite_tint(star: &SystemBody, cam: UPos, occluders: &Query<(&SystemBody, Option<&crate::terrain::Terrain>)>) -> Vec3 {
    let rel = star.pos.delta_m(cam);
    let dist = rel.length();
    if dist <= star.radius_m {
        return Vec3::ONE;
    }
    let dir = rel / dist;
    let ang_r = (star.radius_m / dist).asin().max(1e-9);
    let mut visible = 1.0;
    let mut tr = DVec3::ONE;
    for (b, t) in occluders {
        if b.star || b.radius_m <= 0.0 {
            continue;
        }
        let o = b.pos.delta_m(cam);
        let d_o = o.length();
        if d_o <= b.radius_m {
            // Below the body's radius: its horizon plane hides the lower sky.
            if dir.dot(-o / d_o.max(1e-9)) < 0.0 {
                visible = 0.0;
            }
        } else if d_o < dist {
            let sep = (o / d_o).angle_between(dir);
            visible *= 1.0 - crate::terrain::disc_overlap(ang_r, (b.radius_m / d_o).asin(), sep);
        }
        if let Some(a) = t.and_then(|t| t.atmosphere.as_ref()) {
            let km = a.params.radius_ground as f64 / b.radius_m;
            tr *= crate::atmosphere::transmittance_to_space(&a.params, -o * km, dir);
        }
        if visible <= 0.0 {
            return Vec3::ZERO;
        }
    }
    (tr * visible).as_vec3()
}

fn update_star_spheres(mut q: Query<(&SystemBody, &mut Visibility), With<StarSphere>>) {
    for (b, mut vis) in &mut q {
        *vis = if b.px_radius > 0.4 { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// The sky shells: placed with their bodies, lit by the body's lights (the eclipses and the rings'
/// shadow per fragment in `sky.wesl`, SE's sky.glsl), and drawn at least `AtmoMinWidthPix` thick
/// from afar (SE's FUN_14044fe70).
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_sky_shells(
    cam: Res<FlyCamera>,
    bodies: Query<(&SystemBody, &BodyLighting, Option<&crate::terrain::Terrain>, Option<&crate::eclipse::EclipseCasters>)>,
    mut shells: Query<(&SkyOf, &crate::atmosphere::SkyShell, &mut UPosition, &mut UOrientation, &mut MeterScale, &mut Visibility)>,
    mut mats: ResMut<Assets<crate::atmosphere::SkyMaterial>>,
    rings: Query<&crate::rings::Rings>,
    ring_mats: Res<Assets<crate::rings::RingMaterial>>,
    switches: Res<crate::switches::RenderSwitches>,
) {
    for (sky, shell, mut pos, mut rot, mut scale, mut vis) in &mut shells {
        let Ok((body, light, terrain, casters)) = bodies.get(sky.body) else { continue };
        pos.0 = body.pos;
        rot.0 = body.orient;
        let show = body.px_radius > 0.5 && switches.atmospheres;
        *vis = if show { Visibility::Visible } else { Visibility::Hidden };
        if !show {
            continue;
        }
        let star_dist = light.star_pos.delta_m(body.pos).length().max(1.0);
        let ang = (light.star_radius_m / star_dist) as f32;
        let eclipse = casters.map_or_else(Default::default, |c| c.lights);
        let cam_rel = cam.pos.delta_m(body.pos) / body.radius_m;
        // The shell paints an analytic sun disc over anything behind it, so mask the part covered by
        // an eclipsing body (the largest one in front of the star, seen from the camera) while the
        // camera (or the ground below it) is in a shadow.
        let p = if cam_rel.length() < 1.0 { cam_rel.normalize_or(DVec3::Y) } else { cam_rel };
        let in_shadow = terrain.is_some_and(|t| t.eclipse.light_at(p, light.sun_dir.as_dvec3()).max_element() < 0.999);
        let main = &eclipse[0];
        let occluder = main
            .casters
            .iter()
            .take(main.info.y as usize)
            .filter(|_| switches.eclipses && in_shadow)
            .map(|o| {
                let v = o.truncate().as_dvec3() - cam_rel;
                let r = (o.w as f64 / v.length()).min(1.0).asin();
                v.normalize_or(DVec3::Y).as_vec3().extend(r as f32)
            })
            .filter(|o| o.truncate().dot(light.sun_dir) > (o.w + ang).cos() && o.truncate().dot(light.sun_dir) > 0.0)
            .max_by(|a, b| a.w.total_cmp(&b.w))
            .unwrap_or(Vec4::ZERO);

        // SE's atmosphere at a distance: the drawn height set to AtmoMinWidthPix (at most 0.05 R), its
        // light scaled to match; the eye's distance for the scattering capped at AtmoDistanceLimit radii.
        let gpu = terrain.and_then(|t| t.atmosphere.as_ref());
        let (mut bright, mut atm, mut top_m) = (1.0f32, None, None);
        if let Some(gpu) = gpu {
            let d = cam.pos.delta_m(body.pos).length().max(body.radius_m * 1.0001);
            let px_per_rad = body.px_radius * (d * d - body.radius_m * body.radius_m).sqrt() / body.radius_m;
            let min_width = (switches.atmo_min_width_px as f64 / px_per_rad.max(1e-9)).tan() * d;
            let height = gpu.top_radius_m(body.radius_m) - body.radius_m;
            let forward = cam.rot * DVec3::NEG_Z;
            let cos_view = forward.dot(cam.pos.delta_m(body.pos) / d);
            let (drawn, b) = crate::atmosphere::widened_height(height, body.radius_m, min_width, cos_view);
            if (drawn - height).abs() > height * 1e-4 {
                let wide = gpu.with_height(body.radius_m, drawn);
                atm = Some(wide.uniform());
                top_m = Some(wide.top_radius_m(body.radius_m));
                bright = b as f32;
            } else {
                atm = Some(gpu.uniform());
                top_m = Some(gpu.top_radius_m(body.radius_m));
            }
        }
        if let Some(top) = top_m {
            let want = DVec3::splat(top) * body.shape;
            if scale.0 != want {
                scale.0 = want;
            }
        }
        let top = top_m.unwrap_or(body.radius_m * 1.02);
        // The rings' shadow on the sky (AtmoRingsShadow).
        let ring = rings.iter().find(|r| r.body == sky.body).and_then(|r| ring_mats.get(&r.material));
        let ring_u = ring
            .filter(|_| switches.sky_rings_shadows)
            .map_or(Vec4::ZERO, |m| m.u.geom.truncate().extend(if m.u.params.z > 0.5 { 1.0 } else { 0.5 }));
        let limit = switches.atmo_distance_limit as f64 * body.radius_m / top;
        crate::atmosphere::sky_shadow_update(
            &mut mats,
            shell,
            (ring_u, ring.map(|m| &m.texture)),
            Vec4::new((body.radius_m / top) as f32, switches.eclipse_intensity(), limit as f32, if switches.atmo_extinction { 1.0 } else { 0.0 }),
            eclipse,
            atm,
        );
        crate::atmosphere::sky_uniform_update(
            &mut mats,
            shell,
            light.sun_dir,
            // SE's scattered light: the star's times `Bright` (in the shader) x hdrAtmoScatterBright.
            light.sun_color * light.sun_intensity * crate::terrain::ATM_SCATTER_SCALE * bright,
            ang,
            occluder,
            light.sun_color * light.sun_intensity * crate::terrain::ATM_SUN_SCALE,
            {
                let (dirs, mut colors) = light.extra_uniforms(false);
                for c in &mut colors {
                    *c *= crate::terrain::ATM_SCATTER_SCALE * bright;
                }
                (dirs, colors)
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Star surface material: emissive sphere with limb darkening.

use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// SE's star surface (`tg_sun_glow` in `star_surface.wesl`), from the slots its surface generator
/// fills for stars (`surface_gen::star_surface`, named as tg_common.glh's `sun*` defines).
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct StarSurfaceUniform {
    /// rgb SE's surface brightness at the disc centre's mean temperature, w 1.
    pub color: Vec4,
    /// x Teff (K), y SE's limb darkening amount, z SE's spot temperature (relative to Teff).
    pub temp: Vec4,
    /// xyz SE's Randomize.
    pub randomize: Vec4,
    /// x sunFlowDistFreq, y sunGranMagn, z sunGranFreq, w sunGranOctaves.
    pub gran: Vec4,
    /// x sunGranTempDistort, y sunGranBotTemp, z sunGranTopTemp.
    pub gran2: Vec4,
    /// x sunSpotSqrtDensity, y sunSpotFreq, z sunSpotFilamentFreq.
    pub spots: Vec4,
    /// x sunSpotMaxLat, y sunSpotMinLat.
    pub spots2: Vec4,
}

/// SE's `SurfParams1.y` for stars (the amount of its 0.86x..1.1x temperature limb darkening); the
/// value SE uploads is not decoded, so the full effect is assumed.
const STAR_LIMB_DARKENING: f32 = 1.0;

/// main-def.cfg `hdrStarSurfaceBright` (HDR photo mode, the port's only mode).
pub const HDR_STAR_SURFACE_BRIGHT: f64 = 2000.0;

/// SE's Planck table colour of a blackbody at `t` K: the spectrum from 380 to 780 nm in 5 nm steps
/// through the CIE 1931 matching functions (multi-lobe Gaussian fit) to linear sRGB, scaled so that
/// B(555 nm, 5700 K) = 1.
pub fn planck_table_rgb(t: f64) -> [f64; 3] {
    let lobe = |l: f64, mu: f64, s1: f64, s2: f64| {
        let x = (l - mu) / if l < mu { s1 } else { s2 };
        (-0.5 * x * x).exp()
    };
    let planck = |lm: f64, t: f64| 1.191_042_805_64e-16 / (lm.powi(5) * ((0.014_387_751_9 / (lm * t)).exp() - 1.0));
    let n = 1.0 / planck(555e-9, 5700.0);
    let mut xyz = [0.0; 3];
    for k in 0..81 {
        let l = 380.0 + 5.0 * k as f64;
        let b = planck(l * 1e-9, t) * n;
        xyz[0] += b * (1.056 * lobe(l, 599.8, 37.9, 31.0) + 0.362 * lobe(l, 442.0, 16.0, 26.7) - 0.065 * lobe(l, 501.1, 20.4, 26.2));
        xyz[1] += b * (0.821 * lobe(l, 568.8, 46.9, 40.5) + 0.286 * lobe(l, 530.9, 16.3, 31.1));
        xyz[2] += b * (1.217 * lobe(l, 437.0, 11.8, 36.0) + 0.681 * lobe(l, 459.0, 26.0, 13.8));
    }
    let [x, y, z] = xyz;
    [
        (3.2406 * x - 1.5372 * y - 0.4986 * z).max(0.0),
        (-0.9689 * x + 1.8758 * y + 0.0415 * z).max(0.0),
        (0.0557 * x - 0.2040 * y + 1.0570 * z).max(0.0),
    ]
}

/// SE's star surface brightness in HDR mode (FUN_1404c4090, uploaded as SurfParams4.w by
/// FUN_1404ae4a0, planet.glsl:1310): hdrStarSurfaceBright x 10^((3 - 2s) s^2 (5931.23 / T + 2.37526
/// - 0.852509 ln T)), s = clamp((T - 1200) / 300, 0, 1), T clamped to 808.86..1e5 K; it multiplies the
/// Planck table colour. SE adds an offset lerped from renderer +0x35c0 / +0x35c4, which only its
/// constructor and FUN_140258200 set (to 0).
pub fn star_surface_gain(teff: f64) -> f64 {
    let t = teff.clamp(808.86, 1.0e5);
    let s = ((t - 1200.0) / 300.0).clamp(0.0, 1.0);
    HDR_STAR_SURFACE_BRIGHT * 10f64.powf((3.0 - 2.0 * s) * s * s * (5931.23 / t + 2.375_26 - 0.852_509 * t.ln()))
}

impl StarSurfaceUniform {
    pub fn new(teff: f32, g: &se_core::surface_gen::SurfaceGen) -> Self {
        use se_core::surface_gen as sg;
        let v = |i: usize| g.v(i);
        // GlowMapSun's spotTemp: sunGranBotTemp x clamp(1.27944 x 0.999889^Teff, 0.1, 0.95), so
        // umbrae are darker on hotter stars.
        let spot = v(sg::TROPIC_LATITUDE) * (1.27944 * 0.999889f32.powf(teff)).clamp(0.1, 0.95);
        // SE's brightness at the disc centre's mean temperature (`star_surface.wesl` tc); the shader
        // carries it to each pixel's temperature.
        let (bot, top) = (v(sg::TROPIC_LATITUDE) as f64, v(sg::ICECAP_LATITUDE) as f64);
        let tc = teff as f64 * 1.1 * (bot + (top - bot) * 0.3);
        let rgb = planck_table_rgb(tc).map(|c| (c * star_surface_gain(teff as f64)) as f32);
        Self {
            color: Vec4::new(rgb[0], rgb[1], rgb[2], 1.0),
            temp: Vec4::new(teff, STAR_LIMB_DARKENING, spot, 0.0),
            randomize: Vec4::new(v(0), v(1), v(2), 0.0),
            gran: Vec4::new(v(sg::COLOR_DIST_FREQ), v(sg::HILLS_MAGN), v(sg::HILLS_FREQ), v(sg::CRACKS_OCTAVES).clamp(0.0, 15.0)),
            gran2: Vec4::new(v(sg::CANYONS_MAGN), v(sg::TROPIC_LATITUDE), v(sg::ICECAP_LATITUDE), 0.0),
            // The shader's mareSqrtDensity is the square root of the slot, as for maria.
            spots: Vec4::new(v(sg::MARE_DENSITY).max(0.0).sqrt(), v(sg::MARE_FREQ), v(sg::MONTES_FREQ), 0.0),
            spots2: Vec4::new(v(sg::CLIMATE_POLE), v(sg::CLIMATE_TROPIC), 0.0, 0.0),
        }
    }
}

#[cfg(test)]
mod star_surface_tests {
    use super::*;

    fn luma(t: f64, teff: f64) -> f64 {
        let c = planck_table_rgb(t);
        (0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]) * star_surface_gain(teff)
    }

    #[test]
    fn star_discs_follow_ses_brightness_chain() {
        // The Sun: about 7.1 at the disc centre (1.1 Teff), 2.3 at the limb (0.86 Teff).
        assert!((luma(5778.0 * 1.1, 5778.0) - 7.11).abs() < 0.05, "{}", luma(5778.0 * 1.1, 5778.0));
        assert!((luma(5778.0 * 0.86, 5778.0) - 2.3).abs() < 0.1);
        assert!((luma(3000.0 * 1.1, 3000.0) - 5.54).abs() < 0.05);
        assert!((luma(30000.0 * 1.1, 30000.0) - 2.10).abs() < 0.05);
        // The table is 1 at 555 nm and 5700 K, so a solar blackbody is near white.
        let c = planck_table_rgb(5778.0 * 1.1);
        assert!(c[0] / c[2] > 0.95 && c[0] / c[2] < 1.15);
    }
}

/// SE's 64^3 noise texture (`Noise3DSampler`), read with `textureLoad` by the star surfaces.
fn star_noise_image() -> Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let n = se_core::tg::NOISE3D_SIZE as u32;
    Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: n },
        TextureDimension::D3,
        se_core::tg::noise3d(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct StarSurfaceMaterial {
    #[uniform(0)]
    pub u: StarSurfaceUniform,
    #[texture(1, dimension = "3d")]
    pub noise: Handle<Image>,
}

impl Material for StarSurfaceMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/star_surface.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/star_surface.wesl".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planet_light_fades_by_magnitude() {
        let f = LightingFade::from_main_def("Lighting\n{\n\tPlanetLightingFadeBeginMagn -7\n\tPlanetLightingFadeEndMagn -5\n\tRingsLightingFadeEndMagn -10\n}\n");
        assert_eq!(f, LightingFade::default());
        // The Sun at 1 AU is -26.74; -7 is 1.27e-8 of that (some 9000 AU away).
        assert!((LightingFade::flux(-7.0) / 1.271e-8 - 1.0).abs() < 0.01);
        // The brightest star lights a planet fully wherever it is brighter than -7.
        for flux in [1000.0, 1.0, 1e-3, 2e-8] {
            assert_eq!(f.planet_intensity(flux), 1.0, "{flux}");
        }
        assert_eq!(f.planet_intensity(1e-9), 0.0);
        assert_eq!(f.planet_intensity(0.0), 0.0);
        let mid: Vec<f32> = [3e-9, 6e-9, 1e-8].iter().map(|&x| f.planet_intensity(x)).collect();
        assert!(mid[0] > 0.0 && mid[0] < mid[1] && mid[1] < mid[2] && mid[2] < 1.0, "{mid:?}");
        // Rings lose the star from -10 (2e-7).
        assert!(f.rings_lit(1e-6) && !f.rings_lit(1e-7));
    }

    #[test]
    fn several_stars_are_normalised_to_the_brightest() {
        let f = LightingFade::default();
        // A companion a quarter as bright shows at 0.25^0.35; a star past the fade end, and every
        // one after it, is dropped.
        let k = f.planet_intensities(&[(4.0, true), (1.0, true), (1e-9, true), (1e-10, true)]);
        assert_eq!(k.len(), 2);
        assert_eq!(k[0], 1.0);
        assert!((k[1] - 0.25f32.powf(0.35)).abs() < 1e-6, "{k:?}");
        // Under a faint brightest star the normalisation is to the fade start, as for one star.
        let faint = 2.0 * LightingFade::flux(-7.0);
        assert_eq!(f.planet_intensities(&[(faint, true), (faint, true)]), vec![1.0, 1.0]);
        // Planet shine never sets the scale: 1e-4 of the Sun's light shows at 1e-4^0.35.
        let k = f.planet_intensities(&[(1.0, true), (1e-4, false)]);
        assert!((k[1] - 1e-4f32.powf(0.35)).abs() < 1e-6, "{k:?}");
        // Alone it is scaled to the fade start, as SE does (the port always has a star first).
        assert!((f.planet_intensities(&[(1e-4, false)])[0] - (1e-4 / LightingFade::flux(-7.0)).powf(0.35) as f32).abs() < 1e-3);
        assert!(f.planet_intensities(&[]).is_empty());
    }

    #[test]
    fn illumination_colour() {
        assert!((auto_illum_saturation(2000.0) - 0.874).abs() < 0.01);
        assert_eq!(auto_illum_saturation(6000.0), 0.0);
        assert!((auto_illum_saturation(6700.0) - 0.2427).abs() < 0.01 && (auto_illum_saturation(6701.0) - 0.25).abs() < 0.01);
        assert!((auto_illum_saturation(10000.0) - 0.447).abs() < 0.01);
        assert_eq!(auto_illum_saturation(30000.0), 0.75);
        let mut s = StarSettings::default();
        assert!(illum_color(Vec3::ONE, 3000.0, &s).abs_diff_eq(Vec3::ONE, 1e-6));
        // A red dwarf's light keeps its hue at a lower saturation; without auto, SunIllumSaturation.
        let red = Vec3::new(1.0, 0.5, 0.3);
        let c = illum_color(red, 3000.0, &s);
        assert!(c.x > c.y && c.y > c.z && c.x < 1.0 && c.z > 0.3, "{c}");
        s.illum_saturation_auto = false;
        s.illum_saturation = 1.0;
        assert!(illum_color(red, 3000.0, &s).abs_diff_eq(red, 1e-6));
    }
}
