//! Standalone atmosphere demo / visual test.
//!
//! atmo_demo [SE_INSTALL_DIR] [--view space|ground|sunset] [--screenshot FILE.png] [--model Earth]
//!           [--size WxH] [--alt KM] [--pitch DEG] [--yaw DEG] [--sun DEG] [--scale K] [--irr X] [--frames N]
//!
//! `--scale K` multiplies the planet translation and radius by K (like the engine's scaled space) to prove
//! the shaders only depend on the shell's own transform.

#![allow(dead_code)]

#[path = "../atmosphere.rs"]
mod atmosphere;

use std::path::PathBuf;

use bevy::asset::RenderAssetUsages;
use bevy::camera::Hdr;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy::render::view::Msaa;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::shader::ShaderRef;
use bevy::window::PresentMode;
use se_core::vfs::Vfs;

use atmosphere::{AtmosphereGpu, AtmospherePlugin, SkyMaterial, SkyUniform, load_atmosphere, spawn_sky_shell};

const DEFAULT_SE_PATH: &str = "D:/SteamLibrary/steamapps/common/SpaceEngine";
const EARTH_RADIUS_M: f64 = 6_378_000.0;
const SUN_ANGULAR_RADIUS: f32 = 0.00465;

#[derive(Resource, Clone)]
struct DemoArgs {
    se_path: PathBuf,
    view: String,
    model: String,
    screenshot: Option<PathBuf>,
    frames: u32,
    width: u32,
    height: u32,
    alt_km: Option<f64>,
    pitch: Option<f32>,
    yaw: f32,
    sun_deg: Option<f32>,
    k: f64,
    irradiance: f32,
}

impl DemoArgs {
    fn parse() -> Self {
        let mut a = DemoArgs {
            se_path: PathBuf::from(DEFAULT_SE_PATH),
            view: "space".into(),
            model: "Earth".into(),
            screenshot: None,
            frames: 60,
            width: 1280,
            height: 720,
            alt_km: None,
            pitch: None,
            yaw: 0.0,
            sun_deg: None,
            k: 1.0,
            irradiance: 10.0,
        };
        let mut it = std::env::args().skip(1).peekable();
        let mut first = true;
        while let Some(arg) = it.next() {
            let mut val = || it.next().unwrap_or_default();
            match arg.as_str() {
                "--view" => a.view = val(),
                "--model" => a.model = val(),
                "--screenshot" => a.screenshot = Some(PathBuf::from(val())),
                "--frames" => a.frames = val().parse().unwrap_or(60),
                "--size" => {
                    let s = val();
                    if let Some((w, h)) = s.split_once('x') {
                        a.width = w.parse().unwrap_or(a.width);
                        a.height = h.parse().unwrap_or(a.height);
                    }
                }
                "--alt" => a.alt_km = val().parse().ok(),
                "--pitch" => a.pitch = val().parse().ok(),
                "--yaw" => a.yaw = val().parse().unwrap_or(0.0),
                "--sun" => a.sun_deg = val().parse().ok(),
                "--scale" => a.k = val().parse().unwrap_or(1.0),
                "--irr" => a.irradiance = val().parse().unwrap_or(10.0),
                s if first && !s.starts_with("--") => a.se_path = PathBuf::from(s),
                other => eprintln!("ignoring argument {other}"),
            }
            first = false;
        }
        a
    }
}

fn asset_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("assets");
            if p.join("shaders").is_dir() {
                return p;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

// ------------------------------------------------------------------------------------------------
// Demo ground material (ray-traced sphere that calls atm_ground_atmosphere)
// ------------------------------------------------------------------------------------------------

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct GroundMaterial {
    #[uniform(0)]
    uniform: SkyUniform,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    inscatter: Handle<Image>,
    #[texture(3)]
    transmittance: Handle<Image>,
    #[texture(4)]
    irradiance: Handle<Image>,
}

impl Material for GroundMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/atmo_demo_ground.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/atmo_demo_ground.wesl".into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
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

// ------------------------------------------------------------------------------------------------

#[derive(Resource)]
struct Shot {
    path: Option<PathBuf>,
    frames: u32,
    count: u32,
    taken: bool,
}

fn main() {
    let args = DemoArgs::parse();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: Some(Backends::VULKAN),
                    ..default()
                })),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "atmo_demo".into(),
                    resolution: (args.width, args.height).into(),
                    present_mode: PresentMode::AutoVsync,
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                file_path: asset_dir().to_string_lossy().into_owned(),
                ..default()
            }),
    )
    .add_plugins((AtmospherePlugin, MaterialPlugin::<GroundMaterial>::default()))
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(Shot {
        path: args.screenshot.clone(),
        frames: args.frames,
        count: 0,
        taken: false,
    })
    .insert_resource(args)
    .add_systems(Startup, setup)
    .add_systems(Update, screenshot_system);
    app.run();
}

fn setup(
    mut commands: Commands,
    args: Res<DemoArgs>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sky_materials: ResMut<Assets<SkyMaterial>>,
    mut ground_materials: ResMut<Assets<GroundMaterial>>,
) {
    let vfs = Vfs::mount_install(&args.se_path).expect("mount SpaceEngine install");
    let gpu: AtmosphereGpu = load_atmosphere(&vfs, &args.model, &mut images).expect("load atmosphere model");
    info!("atmosphere {}: {:?}", args.model, gpu.params);

    let r_m = EARTH_RADIUS_M;
    let top_m = gpu.top_radius_m(r_m);
    let height_m = top_m - r_m;
    let k = args.k;

    // Defaults per view: (altitude km, pitch deg, sun elevation deg at the camera, yaw deg).
    let (def_alt, def_pitch, def_sun) = match args.view.as_str() {
        "ground" => (0.3, 25.0, 60.0),
        "sunset" => (0.3, 4.0, 1.5),
        _ => (12000.0, 0.0, 0.0),
    };
    let alt_km = args.alt_km.unwrap_or(def_alt);
    let pitch = args.pitch.unwrap_or(def_pitch).to_radians();
    let yaw = args.yaw.to_radians();
    let space = args.view == "space";

    // Sun direction in world space. Ground views: elevation above the local horizon (+y is up), azimuth
    // along -z. Space view: the star lies 70 degrees off the direction to the camera (partial phase).
    let sun_dir = if space {
        let a = args.sun_deg.unwrap_or(70.0).to_radians();
        Vec3::new(a.sin(), a.cos(), 0.0).normalize()
    } else {
        let e = args.sun_deg.unwrap_or(def_sun).to_radians();
        Vec3::new(0.0, e.sin(), -e.cos())
    };

    // Camera at the render origin; planet centre below it (floating origin).
    let centre = Vec3::new(0.0, -((r_m + alt_km * 1000.0) * k) as f32, 0.0);
    let cam_tf = if space {
        Transform::IDENTITY.looking_at(centre, Vec3::Z)
    } else {
        // forward: yaw around +y from -z, then pitch up.
        let dir = Quat::from_rotation_y(-yaw) * Vec3::new(0.0, pitch.sin(), -pitch.cos());
        Transform::IDENTITY.looking_to(dir, Vec3::Y)
    };
    commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Hdr,
        Tonemapping::AcesFitted,
        Bloom {
            intensity: 0.12,
            ..Bloom::NATURAL
        },
        Msaa::Off,
        Projection::Perspective(PerspectiveProjection {
            fov: if space { 40f32 } else { 70f32 }.to_radians(),
            near: 0.05,
            ..default()
        }),
        cam_tf,
    ));

    let sun_irr = Vec3::splat(args.irradiance);

    // Planet: an oversized-triangle mesh; the ground shader ray-traces the sphere from the transform.
    let tri = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[-1.0f32, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]]);
    let ground = ground_materials.add(GroundMaterial {
        uniform: SkyUniform {
            occluder: Vec4::ZERO,
            disc_irradiance: Vec4::ONE,
            atm: gpu.uniform(),
            sun_dir: sun_dir.extend(0.0),
            sun_irradiance: sun_irr.extend(1.0),
            light_dir: [Vec4::ZERO; 3],
            light_irradiance: [Vec4::ZERO; 3],
            ring: Vec4::ZERO,
            shadow: Vec4::ZERO,
            eclipse: Default::default(),
        },
        inscatter: gpu.inscatter.clone(),
        transmittance: gpu.transmittance.clone(),
        irradiance: gpu.irradiance.clone(),
    });
    commands.spawn((
        Mesh3d(meshes.add(tri)),
        MeshMaterial3d(ground),
        Transform::from_translation(centre).with_scale(Vec3::splat((r_m * k) as f32)),
        Visibility::default(),
        NoFrustumCulling,
    ));

    // Sky shell.
    let shell = spawn_sky_shell(&mut commands, &mut meshes, &mut sky_materials, &gpu, r_m, height_m, 1.0, atmosphere::ATM_LOOK_DEFAULT);
    commands.entity(shell).insert(
        Transform::from_translation(centre).with_scale(Vec3::splat((top_m * k) as f32)),
    );
    // sun parameters (shell material is the only SkyMaterial)
    let handles: Vec<_> = sky_materials.iter().map(|(id, _)| id).collect();
    for id in handles {
        if let Some(mut m) = sky_materials.get_mut(id) {
            m.set_sun(sun_dir, sun_irr, SUN_ANGULAR_RADIUS);
        }
    }
}

fn screenshot_system(mut commands: Commands, mut shot: ResMut<Shot>, mut exit: MessageWriter<AppExit>) {
    let Some(path) = shot.path.clone() else { return };
    shot.count += 1;
    if !shot.taken && shot.count >= shot.frames {
        shot.taken = true;
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    } else if shot.taken && shot.count >= shot.frames + 20 {
        exit.write(AppExit::Success);
    }
}
