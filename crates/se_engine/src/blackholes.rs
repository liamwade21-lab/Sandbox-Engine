//! Black holes: gravitational lensing and accretion disks.
//!
//! Everything is a single HDR post-process on the main camera (see
//! `assets/shaders/blackhole_lens.wesl` and `docs/blackholes.md`). Each frame this
//! module finds the catalog black holes that are big enough on screen to matter,
//! converts them to camera-space uniforms (direction, distance in Schwarzschild
//! radii, disk plane, disk radii, colour temperature) and attaches them to the
//! camera as a [`BlackHoleLens`] component. The shader then traces light rays
//! through the Schwarzschild geometry for the pixels around each hole.
//!
//! Black holes are not spawned as `SystemBody` entities (their catalog radius is
//! 0), so this plugin evaluates them straight from the catalog and ephemeris.

use bevy::core_pipeline::Core3dSystems;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;
use se_core::body_params::AccretionDiskParams;
use se_core::catalog::ObjectRef;
use se_core::ephemeris::Ephemeris;
use se_core::star::StarKind;
use se_core::units::{METERS_PER_AU, SOLAR_MASS_KG};

use crate::camera::{FlyCamera, MainCamera};
use crate::data::SeData;
use crate::universe::{SimTime, UniverseSet};

/// Maximum number of simultaneously lensed black holes.
const MAX_HOLES: usize = 4;
/// Schwarzschild radius per solar mass: 2 G M / c^2.
const RS_PER_MSUN_M: f64 = 2.953_250e3;
/// Mass assumed for catalog black holes that have none.
const DEFAULT_MASS_SUN: f64 = 10.0;
/// Outer disk radius (r_s) when the catalog gives none.
const DEFAULT_OUTER_RS: f64 = 60.0;
/// Colour temperature (K) assumed when the catalog has no `Temperature`.
const DEFAULT_DISK_TEMP: f64 = 1.0e5;
/// Smallest on-screen influence radius (pixels) worth a lens pass.
const MIN_REGION_PX: f64 = 1.5;

/// Per-hole shader data. Layout must match `Hole` in blackhole_lens.wesl.
#[derive(Clone, Copy, ShaderType, Default, Debug)]
pub struct HoleUniform {
    /// xyz: unit view-space direction camera -> hole, w: camera distance in r_s.
    pub dir_d: Vec4,
    /// xyz: disk normal, w: inner radius in r_s (<= 0 means no disk).
    pub axis_in: Vec4,
    /// xyz: disk azimuth reference axis, w: outer radius in r_s.
    pub xref_out: Vec4,
    /// x: influence radius (rad), y: brightness, z: peak colour temperature (K),
    /// w: turbulence contrast.
    pub params: Vec4,
}

/// Camera component driving the lensing post-process.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, Debug)]
#[extract_app(RenderApp)]
pub struct BlackHoleLens {
    /// x, y: tan(fov/2) horizontally / vertically, z: animation time, w: hole count.
    pub view: Vec4,
    pub holes: [HoleUniform; MAX_HOLES],
}

impl FullscreenMaterial for BlackHoleLens {
    fn fragment_shader() -> ShaderRef {
        "shaders/blackhole_lens.wesl".into()
    }

    // Run before bloom (and tone mapping) so the photon ring and the beamed side
    // of the disk glow.
    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system.in_set(Core3dSystems::EarlyPostProcess)
    }
}

/// Static, camera independent description of one catalog black hole.
struct BhInfo {
    star: u32,
    /// Catalog position (parsecs); only meaningful if `has_pos`.
    pos_pc: DVec3,
    has_pos: bool,
    rs_m: f64,
    disk: Option<DiskInfo>,
    /// Disk normal used when the catalog gives no rotation model.
    default_axis: DVec3,
}

struct DiskInfo {
    inner_rs: f64,
    outer_rs: f64,
    temp_k: f64,
    bright: f64,
    contrast: f64,
}

#[derive(Resource, Default)]
struct BhList {
    n_stars: usize,
    list: Vec<BhInfo>,
}

pub struct BlackHolesPlugin;

impl Plugin for BlackHolesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FullscreenMaterialPlugin::<BlackHoleLens>::default())
            .init_resource::<BhList>()
            .add_systems(
                PostUpdate,
                update_lens.after(UniverseSet::Camera).before(UniverseSet::FloatingOrigin),
            );
    }
}

/// Innermost stable circular orbit of a Kerr hole (prograde), in Schwarzschild radii.
pub fn isco_rs(spin: f64) -> f64 {
    let a = spin.clamp(0.0, 0.998);
    let z1 = 1.0 + (1.0 - a * a).cbrt() * ((1.0 + a).cbrt() + (1.0 - a).cbrt());
    let z2 = (3.0 * a * a + z1 * z1).sqrt();
    let r_g = 3.0 + z2 - ((3.0 - z1) * (3.0 + z1 + 2.0 * z2)).sqrt();
    0.5 * r_g
}

fn hash_unit(seed: u64, salt: u64) -> f64 {
    let mut x = seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Collect every catalog black hole and derive its disk parameters.
fn build_list(cat: &se_core::catalog::Catalog) -> Vec<BhInfo> {
    let mut out = Vec::new();
    for (i, s) in cat.stars.iter().enumerate() {
        // A hole whose parent never resolved has no real position (it would sit on the Sun).
        if s.barycenter || !s.has_position || s.class.kind != StarKind::BlackHole {
            continue;
        }
        let mass_sun = if s.mass_kg > 0.0 { s.mass_kg / SOLAR_MASS_KG } else { DEFAULT_MASS_SUN };
        let rs_m = RS_PER_MSUN_M * mass_sun;
        let spin = s.block.f64("KerrSpin").unwrap_or(0.0).abs();
        let disk = s
            .block
            .sub("AccretionDisk")
            .filter(|_| !s.block.bool("NoAccretionDisk").unwrap_or(false))
            .map(|b| disk_info(&AccretionDiskParams::from_block(b), rs_m, spin));
        let seed = se_core::proc_surface::name_seed(&s.names[0]);
        // Deterministic tilt for holes without a rotation model: 30-70 degrees off the pole.
        let tilt = (30.0 + 40.0 * hash_unit(seed, 1)).to_radians();
        let az = std::f64::consts::TAU * hash_unit(seed, 2);
        let default_axis = DVec3::new(tilt.sin() * az.cos(), tilt.cos(), tilt.sin() * az.sin());
        out.push(BhInfo {
            star: i as u32,
            pos_pc: s.pos_pc,
            has_pos: s.has_position,
            rs_m,
            disk,
            default_axis,
        });
    }
    out
}

fn disk_info(p: &AccretionDiskParams, rs_m: f64, spin: f64) -> DiskInfo {
    let isco = isco_rs(spin).max(2.0);
    let cat_inner = if p.inner_radius_km > 0.0 {
        p.inner_radius_km * 1e3 / rs_m
    } else {
        p.inner_radius * METERS_PER_AU / rs_m
    };
    let inner_rs = cat_inner.max(isco);
    let cat_outer = if p.outer_radius_km > 0.0 {
        p.outer_radius_km * 1e3 / rs_m
    } else {
        p.outer_radius * METERS_PER_AU / rs_m
    };
    let outer_rs = if cat_outer > inner_rs * 1.5 { cat_outer } else { (DEFAULT_OUTER_RS).max(inner_rs * 8.0) };
    // The physical inner temperature (1e5 K and up) would be white-blue for all
    // holes; compress it into a display colour temperature.
    let t_phys = if p.temperature > 0.0 { p.temperature } else { DEFAULT_DISK_TEMP };
    let temp_k = (4000.0 * (t_phys / 1.0e4).powf(0.3)).clamp(3500.0, 14_000.0);
    // Catalog `Brightness` spans 1e-6 .. 2500; keep it within a displayable range.
    let bright = p.brightness.max(1e-9).powf(0.1).clamp(0.35, 2.0);
    let contrast = if p.disk_noise_contrast > 0.0 { p.disk_noise_contrast } else { 0.7 };
    DiskInfo {
        inner_rs,
        outer_rs,
        temp_k,
        bright,
        contrast,
    }
}

#[allow(clippy::too_many_arguments)]
fn update_lens(
    mut commands: Commands,
    data: Res<SeData>,
    sim: Res<SimTime>,
    cam: Res<FlyCamera>,
    time: Res<Time>,
    windows: Query<&Window>,
    cams: Query<(Entity, Has<BlackHoleLens>), With<MainCamera>>,
    mut list: ResMut<BhList>,
) {
    let Some((cam_e, has_lens)) = cams.iter().next() else { return };
    if std::env::var_os("SE_NO_BLACKHOLES").is_some() {
        return;
    }
    let cat = &data.catalog;
    if list.n_stars != cat.stars.len() {
        list.n_stars = cat.stars.len();
        list.list = build_list(cat);
    }
    let (w, h) = windows
        .iter()
        .next()
        .map(|w| (w.width() as f64, w.height() as f64))
        .unwrap_or((1280.0, 720.0));
    let tan_y = (cam.fov_deg.to_radians() as f64 * 0.5).tan();
    let tan_x = tan_y * w / h;
    let px_ang = 2.0 * tan_y / h;
    let cam_pc = cam.pos.to_parsecs();
    let inv_rot = cam.rot.inverse();
    let eph = Ephemeris::new(cat);
    let ecl_north = se_core::coords::ecliptic_to_universe(DVec3::Z);

    let mut found: Vec<(f64, HoleUniform)> = Vec::new();
    for info in &list.list {
        if info.has_pos && info.pos_pc.distance(cam_pc) > 2.0 {
            continue;
        }
        let obj = ObjectRef::Star(info.star);
        let delta = eph.position(obj, sim.jd).delta_m(cam.pos);
        let dist = delta.length();
        let d_rs = dist / info.rs_m;
        if d_rs < 1.5 || dist <= 0.0 {
            continue;
        }
        // Angular reach of the effect: several Einstein radii, the shadow and the disk.
        let theta_e = (2.0 / d_rs).sqrt();
        let mut region = (16.0 * theta_e).max((12.0 / d_rs).min(1.5));
        if let Some(dk) = &info.disk {
            let disk_ang = if d_rs <= dk.outer_rs * 1.1 {
                std::f64::consts::PI
            } else {
                1.3 * (dk.outer_rs / d_rs).asin()
            };
            region = region.max(disk_ang);
        }
        if d_rs < 8.0 {
            region = std::f64::consts::PI;
        }
        region = region.min(std::f64::consts::PI);
        if region < MIN_REGION_PX * px_ang {
            continue;
        }

        let dir = (inv_rot * (delta / dist)).as_vec3();
        let (axis_in, xref_out) = match &info.disk {
            Some(dk) => {
                let orient = eph.orientation(obj, sim.jd);
                let pole = (orient * DVec3::Y).normalize();
                let axis_w = if pole.dot(ecl_north) > 0.999_999 { info.default_axis } else { pole };
                let axis = (inv_rot * axis_w.normalize()).normalize();
                let helper = if axis.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
                let xref = axis.cross(helper).normalize();
                (axis.as_vec3().extend(dk.inner_rs as f32), xref.as_vec3().extend(dk.outer_rs as f32))
            }
            None => (Vec4::new(0.0, 1.0, 0.0, 0.0), Vec4::new(1.0, 0.0, 0.0, 0.0)),
        };
        let params = match &info.disk {
            Some(dk) => Vec4::new(region as f32, dk.bright as f32, dk.temp_k as f32, dk.contrast as f32),
            None => Vec4::new(region as f32, 1.0, 6000.0, 0.7),
        };
        found.push((
            region / px_ang,
            HoleUniform {
                dir_d: dir.extend(d_rs as f32),
                axis_in,
                xref_out,
                params,
            },
        ));
    }

    if found.is_empty() {
        if has_lens {
            commands.entity(cam_e).remove::<BlackHoleLens>();
        }
        return;
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut lens = BlackHoleLens {
        view: Vec4::new(tan_x as f32, tan_y as f32, (time.elapsed_secs() % 3600.0) as f32, found.len().min(MAX_HOLES) as f32),
        holes: [HoleUniform::default(); MAX_HOLES],
    };
    for (slot, (_, hole)) in lens.holes.iter_mut().zip(found.iter()) {
        *slot = *hole;
    }
    commands.entity(cam_e).insert(lens);
}
