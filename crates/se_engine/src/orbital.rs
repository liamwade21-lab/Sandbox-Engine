//! A level 4 civilization in space (the vegetation and civilizations plan, step 10): its orbital
//! stations round its home world and its larger moons (`civilization::stations`; the ring and truss
//! designs made in Blender by script, `assets/models/hero/station_*.bin`), drawn with the ship
//! material (`HullMaterial`) and lit by the system's stars, the shadow of the body they orbit
//! included; and its space traffic: streams of ship lights along arcs between the home world, its
//! stations and its colonies (`traffic.wesl`), each station a fixed light from afar. Nothing is built
//! or updated outside a system with a level 4 civilization (`ActiveSystem::home`).

use std::collections::BTreeMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureFormat};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use se_core::catalog::ObjectRef;
use se_core::procgen::civilization::{StationSpec, culture, stations};
use se_core::upos::UPos;

use crate::camera::FlyCamera;
use crate::data::SeData;
use crate::ship_assets::{HullDefaults, HullMaterial, HullTextures, hull_uniform, image_with_mips};
use crate::system::{ActiveSystem, AmbientLighting, BodyLighting, SystemBody, SystemMember};
use crate::universe::{MeterScale, ScaleAnchor, ScaledSpace, SimTime, UOrientation, UPosition, UniverseSet};

/// A station's mesh is drawn within this distance (m) of the camera; beyond it its light alone.
const STATION_DRAW_M: f64 = 3.0e7;
/// The traffic and the stations' lights are drawn within this distance (m) of the home world.
const TRAFFIC_DRAW_M: f64 = 6.0e12;
/// The ring station's spin: a gravity (m/s^2) on its ring of 700 m (times its scale).
const RING_G: f64 = 9.81;
/// The traffic buffer's records (the stations' lights and the streams) at most.
const TRAFFIC_MAX: usize = 160;

/// `SE_ORBITAL=0` (comparisons): no stations or traffic; `s` the stations alone, `t` the traffic
/// and the stations' lights alone.
fn orbital_mode() -> u8 {
    static ON: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_ORBITAL").ok().and_then(|v| v.bytes().next()).unwrap_or(b'1'))
}

fn orbital_on() -> bool {
    orbital_mode() != b'0'
}

pub struct OrbitalPlugin;

impl Plugin for OrbitalPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<TrafficMaterial>::default())
            .init_resource::<Orbital>()
            .add_systems(PostUpdate, update_orbital.after(UniverseSet::Camera).before(UniverseSet::FloatingOrigin));
    }
}

/// A stream's record in `traffic.wesl`'s storage buffer.
#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct TrafficUniform {
    /// xyz the start (camera-relative m), w the ships' count.
    pub a: Vec4,
    /// xyz the end, w the cycles a second.
    pub b: Vec4,
    /// xyz the arc's control point.
    pub c: Vec4,
    /// rgb the lights' colour, w their brightness.
    pub color: Vec4,
    /// x the clock (s), y a ship's size (m), z its least size (pixels), w 1 for a fixed light.
    pub misc: Vec4,
    /// x, y the scaled space's factor at the bodies the start and the end belong to (their centres':
    /// the points shrink with the bodies they meet, as `apply_floating_origin` draws those).
    pub k: Vec4,
}

/// A stream's material: its record's index (x) in the shared buffer of records, both fixed (the
/// records are written in place each frame: a material changed every frame is prepared again).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct TrafficMaterial {
    #[uniform(0)]
    pub slot: UVec4,
    #[storage(1, read_only)]
    pub records: Handle<ShaderBuffer>,
}

impl Material for TrafficMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/traffic.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/traffic.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The dev command `station N KM`'s request: look at station N from KM away.
#[derive(Resource, Default)]
pub struct StationGoto(pub Option<(usize, f64)>);

/// A station's part (a mesh of one of its materials).
#[derive(Component)]
struct StationPart;

/// A stream of ship lights or a station's light.
#[derive(Component)]
struct TrafficLight;

/// One end of a stream: a body (its entity and radius) or a station.
#[derive(Clone, Copy)]
enum End {
    Body(Entity, f64),
    Station(usize),
}

struct Station {
    spec: StationSpec,
    parent: Entity,
    parent_radius: f64,
    /// G M of the body it orbits (m^3/s^2).
    gm: f64,
    parts: Vec<(Entity, Handle<HullMaterial>)>,
    /// Its light from afar and its record.
    light: (Entity, usize),
    pos: UPos,
    /// What its materials were last lit with: the sun's direction, the shadow, the ambient light and
    /// the lights' colours (the sun's, the other lights' summed).
    lit: Option<(Vec3, bool, f32, Vec4)>,
}

struct Stream {
    from: End,
    to: End,
    /// The side its arc bows to (+1, -1) and how far (a share of its length).
    side: f64,
    bow: f64,
    entity: Entity,
    /// Its record in the buffer.
    slot: usize,
    count: u32,
    speed: f32,
    color: Vec3,
}

#[derive(Resource, Default)]
struct Orbital {
    /// The system it was built for, and whether it is.
    root: Option<u32>,
    built: bool,
    home: Option<Entity>,
    stations: Vec<Station>,
    streams: Vec<Stream>,
    /// The two designs' meshes, by material layer, and their materials (once).
    designs: Option<[Vec<(u8, Handle<Mesh>)>; 2]>,
    templates: BTreeMap<u8, HullMaterial>,
    /// The traffic's records (`TRAFFIC_MAX`) and their buffer.
    records: Vec<TrafficUniform>,
    buffer: Option<Handle<ShaderBuffer>>,
}

/// Writes `records` into `buffer` in place (the same size: the buffer and the materials' bind groups
/// stay), only when its bytes change.
fn write_records(buffers: &mut Assets<ShaderBuffer>, buffer: &Handle<ShaderBuffer>, records: &Vec<TrafficUniform>) {
    let bytes = crate::stars::gpu_bytes(records);
    if buffers.get(buffer).and_then(|b| b.cast_slice::<u8>()) == Some(&bytes[..]) {
        return;
    }
    if let Some(mut b) = buffers.get_mut(buffer) {
        match b.cast_slice_mut::<u8>() {
            Some(d) if d.len() == bytes.len() => d.copy_from_slice(&bytes),
            _ => *b = ShaderBuffer::new(bytes, RenderAssetUsages::default()),
        }
    }
}

/// A station design's meshes by material layer (`assets/models/hero/NAME.bin`): real size about its
/// middle, uv in tiles of its material (hull 30 m, solar cells 12 m, habitat 30 m).
fn station_meshes(name: &str) -> Option<Vec<(u8, Mesh)>> {
    let ((p, n, uv, idx), size) = crate::scatter::library::hero_file(name)?;
    let mut by: BTreeMap<u8, (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>)> = BTreeMap::new();
    for &i in &idx {
        let i = i as usize;
        let layer = (uv[i][1] / 10000.0).floor().clamp(0.0, 7.0) as u8;
        let m = [uv[i][0], uv[i][1] - layer as f32 * 10000.0 - 5000.0];
        let tile = match layer {
            6 => 12.0,
            _ => 30.0,
        };
        let e = by.entry(layer).or_default();
        e.0.push([p[i][0] * size[0], (p[i][1] - 0.5) * size[1], p[i][2] * size[2]]);
        e.1.push(n[i]);
        e.2.push([m[0] / tile, m[1] / tile]);
    }
    Some(
        by.into_iter()
            .map(|(layer, (pos, nrm, uvs))| {
                let count = pos.len() as u32;
                let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
                    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
                    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
                    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
                    .with_inserted_indices(Indices::U32((0..count).collect()));
                if let Err(e) = mesh.generate_tangents() {
                    warn!("orbital: tangents of {name} layer {layer}: {e}");
                }
                (layer, mesh)
            })
            .collect(),
    )
}

/// `assets/textures/station/NAME.png` (made with the image generator) with its mips, repeating.
fn station_texture(name: &str) -> Option<Image> {
    let rel = std::path::Path::new("textures").join("station").join(format!("{name}.png"));
    let path = crate::config::asset_file(&rel).unwrap_or_default();
    let img = image::load_from_memory(&std::fs::read(&path).ok()?).ok()?.to_rgba8();
    let img = image::imageops::resize(&img, 512, 512, image::imageops::FilterType::Triangle);
    Some(image_with_mips(img.into_raw(), 512, 512, TextureFormat::Rgba8UnormSrgb))
}

/// The habitats' lit windows: rows of small windows in a 30 m tile, some dark.
fn windows_texture() -> Image {
    const S: u32 = 128;
    let mut px = vec![0u8; (S * S * 4) as usize];
    for y in 0..S {
        for x in 0..S {
            let (row, col) = (y / 8, x / 6);
            let lit = (row.wrapping_mul(7919) ^ col.wrapping_mul(104_729)).wrapping_mul(0x9e37_79b9) >> 29 < 5;
            let win = y % 8 >= 3 && y % 8 < 6 && x % 6 >= 1 && x % 6 < 5 && lit;
            let v = if win { 255 } else { 0 };
            let i = ((y * S + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    image_with_mips(px, S, S, TextureFormat::Rgba8UnormSrgb)
}

/// The stations' material of each layer: hull (4), lights (5, the culture's colour), solar cells
/// (6), habitat (7: hull with lit windows).
fn templates(images: &mut Assets<Image>, defaults: &HullDefaults, light: Vec3) -> BTreeMap<u8, HullMaterial> {
    let hull = station_texture("hull");
    let solar = station_texture("solar");
    if hull.is_none() || solar.is_none() {
        warn!("orbital: station textures missing: flat colours");
    }
    let white = || image_with_mips(vec![255; 4], 1, 1, TextureFormat::Rgba8UnormSrgb);
    let mut out = BTreeMap::new();
    for layer in [4u8, 5, 6, 7] {
        let mut sm = se_core::ship::ShipMaterial::default();
        let mut t = HullTextures::default();
        match layer {
            5 => {
                t.emissive = Some(white());
                sm.emis_color = light.to_array();
                sm.emis_bright = 6.0;
                sm.diff_color = [0.7; 3];
            }
            6 => {
                t.diffuse = solar.clone();
                sm.spec_color = [0.6; 3];
                sm.spec_power = 60.0;
            }
            7 => {
                t.diffuse = hull.clone();
                t.emissive = Some(windows_texture());
                sm.emis_color = [1.0, 0.85, 0.65];
                sm.emis_bright = 1.2;
            }
            _ => {
                t.diffuse = hull.clone();
                sm.spec_color = [0.3; 3];
                sm.spec_power = 40.0;
            }
        }
        let u = hull_uniform(&sm, &t, 0.0, defaults.thermal_shift);
        out.insert(layer, t.into_material(images, defaults, u));
    }
    out
}

/// A stream's mesh: `n` quads, each vertex its ship's index and its corner (`traffic.wesl`).
fn dots_mesh(n: u32) -> Mesh {
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for i in 0..n {
        let b = pos.len() as u32;
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            pos.push([i as f32, x, y]);
        }
        idx.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_indices(Indices::U32(idx))
}

/// The body's brightest star and up to three more as a station's lights, the first in the shadow of
/// the body it orbits when `shadow`.
fn set_station_lights(u: &mut crate::ship_assets::HullUniform, light: &BodyLighting, ambient: f32, shadow: bool) {
    let (dirs, colors) = light.extra_uniforms(false);
    let on = light.sun_intensity > 0.0 && !shadow;
    u.light_dir[0] = light.sun_dir.extend(if on { 1.0 } else { 0.0 });
    u.light_color[0] = (light.sun_color * light.sun_intensity * if shadow { 0.0 } else { 1.0 }).extend(1.0);
    for i in 0..3 {
        u.light_dir[i + 1] = dirs[i];
        u.light_color[i + 1] = colors[i];
    }
    u.ambient.x = ambient;
}

#[allow(clippy::too_many_arguments)]
fn update_orbital(
    mut commands: Commands,
    mut orb: ResMut<Orbital>,
    (active, data, sim, time, gfx): (Res<ActiveSystem>, Res<SeData>, Res<SimTime>, Res<Time>, Res<crate::graphics::GraphicsSettings>),
    (mut cam, space, ambient, defaults, goto): (ResMut<FlyCamera>, Res<ScaledSpace>, Res<AmbientLighting>, Res<HullDefaults>, Option<ResMut<StationGoto>>),
    (mut meshes, mut images): (ResMut<Assets<Mesh>>, ResMut<Assets<Image>>),
    (mut hull_mats, mut traffic_mats, mut buffers): (ResMut<Assets<HullMaterial>>, ResMut<Assets<TrafficMaterial>>, ResMut<Assets<ShaderBuffer>>),
    bodies: Query<(Entity, &SystemBody, &BodyLighting)>,
    mut parts: Query<(&mut UPosition, &mut UOrientation, &mut ScaleAnchor, &mut Visibility), (With<StationPart>, Without<TrafficLight>)>,
    mut lights: Query<&mut Visibility, (With<TrafficLight>, Without<StationPart>)>,
    mut proj: Query<&mut Projection, With<crate::camera::MainCamera>>,
) {
    // A new system: its members (these among them) went with the old one.
    if orb.root != active.root {
        orb.root = active.root;
        orb.built = false;
        orb.home = None;
        orb.stations.clear();
        orb.streams.clear();
    }
    let Some((home_i, seed)) = active.home.filter(|_| orbital_on()) else {
        // (A `station` request with no stations here is dropped.)
        if let Some(mut g) = goto {
            if g.0.take().is_some() {
                info!("station: no level 4 civilization in this system");
            }
        }
        return;
    };
    let cat = &data.catalog;
    let entity_of = |i: u32| bodies.iter().find(|(_, b, _)| b.obj == ObjectRef::Body(i)).map(|(e, b, _)| (e, b.radius_m));
    if !orb.built {
        // Wait for the home world, its moons and the colonies to be spawned.
        let Some((home, home_r)) = entity_of(home_i) else { return };
        let hb = &cat.bodies[home_i as usize];
        let moons: Vec<u32> = cat
            .system_members(&active.name)
            .into_iter()
            .filter_map(|o| match o {
                ObjectRef::Body(i) if hb.names.iter().any(|n| *n == cat.bodies[i as usize].parent) => Some(i),
                _ => None,
            })
            .collect();
        let moon_e: Vec<Option<(Entity, f64)>> = moons.iter().map(|&i| entity_of(i)).collect();
        let colony_e: Vec<Option<(Entity, f64)>> = active.colonies.iter().map(|&i| entity_of(i)).collect();
        if moon_e.iter().chain(&colony_e).any(|e| e.is_none()) {
            return;
        }
        let c = culture(seed, 4);
        let light = Vec3::from_array(c.light);
        if orb.designs.is_none() {
            let mut load = |name: &str| station_meshes(name).unwrap_or_default().into_iter().map(|(l, m)| (l, meshes.add(m))).collect::<Vec<_>>();
            orb.designs = Some([load("station_ring"), load("station_truss")]);
            orb.templates = templates(&mut images, &defaults, light);
        }
        // The traffic's records: one buffer of `TRAFFIC_MAX`, each light and stream a fixed index.
        orb.records = vec![TrafficUniform::default(); TRAFFIC_MAX];
        let records = buffers.add(ShaderBuffer::new(crate::stars::gpu_bytes(&orb.records), RenderAssetUsages::default()));
        orb.buffer = Some(records.clone());
        let mut next_slot = 0usize;
        let g = se_core::units::G;
        let moon_radii: Vec<f64> = moons.iter().map(|&i| cat.bodies[i as usize].radius_km).collect();
        let specs = stations(seed, &moon_radii);
        let designs = orb.designs.clone().unwrap_or_default();
        let mut new_stations = Vec::new();
        for spec in specs {
            let (parent, parent_r, mass) = match spec.moon {
                Some(m) => {
                    let (e, r) = moon_e[m].unwrap();
                    (e, r, cat.bodies[moons[m] as usize].mass_kg)
                }
                None => (home, home_r, hb.mass_kg),
            };
            let mut ps = Vec::new();
            for (layer, mesh) in designs[spec.design.min(1) as usize].iter().filter(|_| orbital_mode() != b't') {
                let Some(t) = orb.templates.get(layer) else { continue };
                let mut t = t.clone();
                // The lights in this civilization's colour (the templates are made once).
                if *layer == 5 {
                    t.u.emis = (light * 6.0).extend(0.0);
                }
                let mat = hull_mats.add(t);
                let e = commands
                    .spawn((
                        SystemMember,
                        StationPart,
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(mat.clone()),
                        UPosition::default(),
                        UOrientation::default(),
                        ScaleAnchor(UPos::default()),
                        MeterScale(DVec3::splat(spec.scale)),
                        Transform::IDENTITY,
                        Visibility::Hidden,
                    ))
                    .id();
                ps.push((e, mat));
            }
            let slot = next_slot.min(TRAFFIC_MAX - 1);
            next_slot += 1;
            let lm = traffic_mats.add(TrafficMaterial { slot: UVec4::new(slot as u32, 0, 0, 0), records: records.clone() });
            let le = commands
                .spawn((SystemMember, TrafficLight, Mesh3d(meshes.add(dots_mesh(1))), MeshMaterial3d(lm), Transform::IDENTITY, NoFrustumCulling, Visibility::Hidden))
                .id();
            new_stations.push(Station { spec, parent, parent_radius: parent_r, gm: g * mass.max(1.0), parts: ps, light: (le, slot), pos: UPos::default(), lit: None });
        }
        // The streams: the home world to each station round it, each moon to its station, the home
        // world to each colony; both ways, bowing to either side.
        let mut new_streams = Vec::new();
        let mut stream = |from: End, to: End, count: u32, speed: f32, color: Vec3, side: f64, bow: f64| {
            if next_slot >= TRAFFIC_MAX {
                return;
            }
            let slot = next_slot;
            next_slot += 1;
            let mat = traffic_mats.add(TrafficMaterial { slot: UVec4::new(slot as u32, 0, 0, 0), records: records.clone() });
            let entity = commands
                .spawn((SystemMember, TrafficLight, Mesh3d(meshes.add(dots_mesh(count))), MeshMaterial3d(mat), Transform::IDENTITY, NoFrustumCulling, Visibility::Hidden))
                .id();
            new_streams.push(Stream { from, to, side, bow, entity, slot, count, speed, color });
        };
        let warm = Vec3::new(1.0, 0.75, 0.45);
        for (k, s) in new_stations.iter().enumerate().filter(|_| orbital_mode() != b's') {
            let parent = End::Body(s.parent, s.parent_radius);
            stream(parent, End::Station(k), 10, 1.0 / 90.0, light, 1.0, 0.25);
            stream(End::Station(k), parent, 10, 1.0 / 90.0, warm, -1.0, 0.25);
        }
        for e in colony_e.iter().flatten().filter(|_| orbital_mode() != b's') {
            let colony = End::Body(e.0, e.1);
            let home_end = End::Body(home, home_r);
            stream(home_end, colony, 30, 1.0 / 600.0, light, 1.0, 0.12);
            stream(colony, home_end, 30, 1.0 / 600.0, warm, -1.0, 0.12);
        }
        info!("orbital: {} stations and {} streams of traffic round {}", new_stations.len(), new_streams.len(), hb.names.first().map_or("", |n| n.as_str()));
        orb.stations = new_stations;
        orb.streams = new_streams;
        orb.home = Some(home);
        orb.built = true;
    }
    let Some(home) = orb.home else { return };
    let Ok((_, home_b, _)) = bodies.get(home) else { return };
    let cam_pos = cam.pos;
    // (The Cities setting Off hides the stations and the traffic.)
    let near = gfx.quality.cities > 0 && cam_pos.delta_m(home_b.pos).length() < TRAFFIC_DRAW_M;
    let t_s = (sim.jd - 2_451_545.0) * 86_400.0;
    let clock = (time.elapsed_secs_f64() % 86_400.0) as f32;
    let rel = |p: UPos| -> Vec3 { p.delta_m(cam_pos).as_vec3() };
    // The scaled space's factor at a body's centre (its drawn size and place: universe.rs).
    let factor = |p: UPos| space.factor(cam_pos.delta_m(p).length()) as f32;
    let mut clear = f64::INFINITY;
    // The stations: on their orbits, spinning (the ring for its gravity) or facing along them.
    let amb = ambient.0;
    let mut orb_records = std::mem::take(&mut orb.records);
    for st in orb.stations.iter_mut() {
        let Ok((_, pb, plight)) = bodies.get(st.parent) else { continue };
        let s = st.spec;
        let r = s.radius * st.parent_radius;
        let theta = s.phase + (st.gm / (r * r * r)).sqrt() * t_s;
        // The orbit's plane fixed in space (the body's pole alone, not its daily spin), prograde (the
        // way the body turns about +y).
        let pole = pb.orient * DVec3::Y;
        let plane = DQuat::from_rotation_arc(DVec3::Y, pole) * DQuat::from_rotation_y(s.node) * DQuat::from_rotation_x(s.tilt);
        let along_orbit = DQuat::from_rotation_y(theta);
        let off = plane * along_orbit * DVec3::new(r, 0.0, 0.0);
        st.pos = pb.pos.offset_m(off);
        let rot = if s.design == 0 {
            plane * DQuat::from_rotation_y((RING_G / (700.0 * s.scale)).sqrt() * (t_s % 1.0e6))
        } else {
            plane * along_orbit
        };
        let d_cam = cam_pos.delta_m(st.pos).length();
        let show = near && d_cam < STATION_DRAW_M;
        if show {
            // Its farthest part from its middle (the ring station's rim and wings: about 750 m).
            clear = clear.min(d_cam - 780.0 * s.scale);
        }
        // The shadow of the body it orbits: behind it from the star, within its radius of the axis.
        let sun = plight.sun_dir.as_dvec3();
        let along = off.dot(sun);
        let shadow = along < 0.0 && (off - sun * along).length() < st.parent_radius;
        let (_, extra_cols) = plight.extra_uniforms(false);
        let lights_key = (plight.sun_color * plight.sun_intensity).extend(extra_cols.iter().map(|c| c.truncate().length()).sum());
        let stale = st.lit.is_none_or(|(d, sh, a, k)| d.dot(plight.sun_dir) < 0.99999 || sh != shadow || a != amb || k.distance(lights_key) > 1e-3);
        for (e, mat) in &st.parts {
            if let Ok((mut up, mut uo, mut anchor, mut vis)) = parts.get_mut(*e) {
                up.0 = st.pos;
                uo.0 = rot;
                // Shrunk with the body it orbits (its centre's factor).
                anchor.0 = pb.pos;
                let want = if show { Visibility::Visible } else { Visibility::Hidden };
                if *vis != want {
                    *vis = want;
                }
            }
            if show && stale {
                if let Some(mut m) = hull_mats.get_mut(mat) {
                    set_station_lights(&mut m.u, plight, amb, shadow);
                }
            }
        }
        if show && stale {
            st.lit = Some((plight.sun_dir, shadow, amb, lights_key));
        }
        // Its light from afar.
        if let Ok(mut v) = lights.get_mut(st.light.0) {
            let want = if near { Visibility::Visible } else { Visibility::Hidden };
            if *v != want {
                *v = want;
            }
        }
        if near {
            let p = rel(st.pos);
            let k = factor(pb.pos);
            orb_records[st.light.1] = TrafficUniform {
                a: p.extend(1.0),
                b: p.extend(0.0),
                c: p.extend(0.0),
                color: Vec3::new(1.0, 0.95, 0.9).extend(3.0),
                misc: Vec4::new(clock, 20.0 * s.scale as f32, 2.0, 1.0),
                k: Vec4::new(k, k, 0.0, 0.0),
            };
        }
    }
    // The near plane in front of the nearest station drawn (as ship.rs does for a ship).
    if clear.is_finite() {
        let want = (clear * 0.5).clamp(0.5, 1000.0) as f32;
        for mut p in &mut proj {
            if let Projection::Perspective(p) = p.as_mut() {
                p.near = p.near.min(want);
            }
        }
    }
    // `station N KM`: the camera KM from station N, between it and the star, a little above its plane.
    if let Some(mut g) = goto {
        if let Some((n, km)) = g.0.take() {
            if let Some(st) = orb.stations.get(n.min(orb.stations.len().saturating_sub(1))) {
                if let Ok((_, pb, plight)) = bodies.get(st.parent) {
                    let up = pb.orient * DVec3::Y;
                    let away = (plight.sun_dir.as_dvec3() * 0.8 + up * 0.35 + st.pos.delta_m(pb.pos).normalize_or(DVec3::X) * 0.4).normalize();
                    cam.pos = st.pos.offset_m(away * km * 1000.0);
                    cam.rot = crate::camera::look_rotation(-away, up);
                    cam.velocity = DVec3::ZERO;
                    cam.follow = None;
                    info!("station: {} of {} ({}), {km} km away", n, orb.stations.len(), if st.spec.design == 0 { "ring" } else { "truss" });
                }
            }
        }
    }
    // The traffic.
    // Each end: its place, the radius it leaves from, and the centre of the body it belongs to.
    let ends: Vec<(UPos, Entity)> = orb.stations.iter().map(|s| (s.pos, s.parent)).collect();
    let end_pos = |e: End| -> Option<(UPos, f64, UPos)> {
        match e {
            End::Body(b, r) => bodies.get(b).ok().map(|(_, sb, _)| (sb.pos, r, sb.pos)),
            End::Station(i) => ends.get(i).and_then(|(p, parent)| bodies.get(*parent).ok().map(|(_, sb, _)| (*p, 0.0, sb.pos))),
        }
    };
    for s in &orb.streams {
        if let Ok(mut v) = lights.get_mut(s.entity) {
            let want = if near && orbital_mode() != b'h' { Visibility::Visible } else { Visibility::Hidden };
            if *v != want {
                *v = want;
            }
        }
        if !near {
            continue;
        }
        let (Some((pa, ra, ca)), Some((pb, rb, cb))) = (end_pos(s.from), end_pos(s.to)) else { continue };
        let ab = pb.delta_m(pa);
        let len = ab.length().max(1.0);
        let dir = ab / len;
        // From just above the one surface to just above the other, bowing out from the home world
        // and to its side.
        let a = pa.offset_m(dir * ra * 1.02);
        let b = pb.offset_m(-dir * rb * 1.02);
        let span = b.delta_m(a);
        let mid = a.offset_m(span * 0.5);
        // Out from the home world across the chord; where the chord runs through the home world, its
        // pole (never the rounding's noise).
        let rel_home = mid.delta_m(home_b.pos);
        let mut perp = rel_home - dir * rel_home.dot(dir);
        if perp.length() < 1e-6 * rel_home.length().max(1.0) {
            let pole = home_b.orient * DVec3::Y;
            perp = pole - dir * pole.dot(dir);
        }
        let out = perp.normalize_or(dir.any_orthonormal_vector());
        let side = dir.cross(out);
        let c = mid.offset_m((out * 0.6 + side * s.side) * span.length() * s.bow);
        orb_records[s.slot] = TrafficUniform {
            a: rel(a).extend(s.count as f32),
            b: rel(b).extend(s.speed),
            c: rel(c).extend(0.0),
            color: s.color.extend(2.5),
            misc: Vec4::new(clock, 25.0, 1.6, 0.0),
            k: Vec4::new(factor(ca), factor(cb), 0.0, 0.0),
        };
    }
    if near {
        if let Some(buffer) = orb.buffer.clone() {
            write_records(&mut buffers, &buffer, &orb_records);
        }
    }
    orb.records = orb_records;
}
