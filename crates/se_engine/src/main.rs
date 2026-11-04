//! SpaceEngine port: Rust + Bevy, rendering through Vulkan.
//!
//! Reads the original game data (catalogs, textures, atmosphere tables) from a
//! SpaceEngine installation at runtime; nothing from the install is copied into
//! this repository.

mod atmosphere;
mod aurora;
mod blackholes;
mod bloom;
mod body_models;
mod brown_dwarf;
mod browser;
mod star_search;
mod camera;
mod capture;
mod clusters;
mod comets;
mod config;
mod constellations;
mod corona;
mod data;
mod exposure;
mod display;
mod eclipse;
mod ellipticals;
mod galaxies;
mod graphics;
mod grids;
mod lensflare;
mod log_file;
mod memory;
mod color_correction;
mod nebula_raymarch;
mod nebulae;
mod hud;
mod i18n;
mod models;
mod mr_assets;
mod music;
mod overlay;
mod perf;
mod pipeline_watch;
mod procgalaxies;
mod projection;
mod procsystems;
mod procstars;
mod rings;
mod rings3d;
mod messages;
mod script_gui;
mod script_vars;
mod scripting;
mod render_scale;
mod screenshot;
mod session;
mod settings;
mod settings_window;
mod ship;
mod ship_assets;
mod skin;
mod sound;
mod splash;
mod stars;
mod surface_tex;
mod switches;
mod system;
mod sysbrowser;
mod terrain;
mod terrain_gen;
mod universe;
mod volumetric_clouds;
mod waypoints;
mod wiki;

use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy_egui::EguiPlugin;

fn main() {
    // Before anything reads or creates the user folder (version 2's first run copies version 1's).
    let user_dir = settings::prepare_user_dir();
    let cfg = config::EngineConfig::from_args();
    // Mount the install and load the catalogs while Bevy builds its plugins (data.rs).
    let data_load = data::DataLoad::start(&cfg);
    let asset_dir = config::asset_dir();
    let display = display::startup_settings(&cfg);

    let mut app = App::new();
    // Before the plugins: the log file reads SE's Log settings from the install.
    app.insert_resource(cfg.clone());
    let task_pools = task_pool_options(stars::main_def_f64(&cfg, "MaxThreads").map_or(0, |v| v.max(0.0) as usize));
    app.add_plugins(
        DefaultPlugins
            .set(TaskPoolPlugin { task_pool_options: task_pools })
            .set(bevy::log::LogPlugin { custom_layer: log_file::layer, ..default() })
            .set(RenderPlugin {
                // The port targets Vulkan only.
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: Some(Backends::VULKAN),
                    ..default()
                })),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(display.window(&cfg, "SpaceEngine (Rust/Bevy port)")),
                ..default()
            })
            .set(AssetPlugin {
                file_path: asset_dir.to_string_lossy().into_owned(),
                ..default()
            }),
    )
    .add_plugins(EguiPlugin::default())
    .add_plugins(display::DisplayPlugin(display))
    .add_plugins((sysbrowser::SysBrowserPlugin, splash::SplashPlugin))
    .insert_resource(ClearColor(Color::BLACK))
    .insert_resource(cfg)
    // Core: game data, simulation clock / floating origin, observer.
    .add_plugins((data::DataPlugin::new(data_load), universe::UniversePlugin, camera::CameraPlugin, memory::MemoryPlugin, exposure::ExposurePlugin, color_correction::ColorCorrectionPlugin, render_scale::RenderScalePlugin))
    // Deep sky (Bevy plugin tuples are limited to 15 entries, hence the groups).
    .add_plugins((
        stars::StarsPlugin,
        procstars::ProcStarsPlugin,
        galaxies::GalaxiesPlugin,
        ellipticals::EllipticalsPlugin,
        procgalaxies::ProcGalaxiesPlugin,
        procsystems::ProcSystemsPlugin,
        nebulae::NebulaePlugin,
        nebula_raymarch::NebulaRaymarchPlugin,
        clusters::ClustersPlugin,
    ))
    // Star systems: bodies, surfaces, atmospheres, rings, models, comets.
    .add_plugins((
        system::SystemPlugin,
        terrain::TerrainPlugin,
        terrain_gen::TerrainGenPlugin,
        surface_tex::SurfaceTexturesPlugin,
        atmosphere::AtmospherePlugin,
        blackholes::BlackHolesPlugin,
        bloom::SeBloomPlugin,
        rings::RingsPlugin,
        models::ModelsPlugin,
        body_models::BodyModelsPlugin,
        comets::CometsPlugin,
        corona::CoronaPlugin,
        aurora::AuroraPlugin,
        eclipse::EclipsePlugin,
        lensflare::LensFlarePlugin,
    ))
    // Volumetric clouds and weather; Megaton Rainfall's assets for the vegetation and cities.
    .add_plugins((volumetric_clouds::VolumetricCloudsPlugin, mr_assets::MrAssetsPlugin))
    // Interface and audio.
    .add_plugins((
        i18n::I18nPlugin,
        skin::SkinPlugin,
        messages::MessagesPlugin,
        hud::HudPlugin,
        wiki::WikiPlugin,
        browser::BrowserPlugin,
        star_search::StarSearchPlugin,
        overlay::OverlayPlugin,
        music::MusicPlugin,
        sound::SoundPlugin,
        settings::SettingsPlugin,
        graphics::GraphicsPlugin,
        scripting::ScriptingPlugin,
        ship::ShipPlugin,
    ))
    // Sky overlays: coordinate grids and constellations.
    .add_plugins((grids::GridsPlugin, constellations::ConstellationsPlugin, switches::SwitchesPlugin))
    // Settings window, screenshots and the saved session.
    .add_plugins((settings_window::SettingsWindowPlugin, screenshot::ScreenshotPlugin, session::SessionPlugin))
    // Offscreen views: the HD screenshot tiles, skybox export and the projection modes' cube faces.
    .add_plugins((capture::CapturePlugin, projection::ProjectionPlugin))
    // Frame-time measurement (SE_PERF=FILE only).
    .add_plugins((perf::PerfPlugin, pipeline_watch::PipelineWatchPlugin))
    .add_systems(Startup, no_light_clustering);
    // Now that the log file is open.
    match user_dir {
        Some(Ok(line)) => info!("{line}"),
        Some(Err(line)) => warn!("{line}"),
        None => {}
    }
    app.run();
}

/// Bevy's task pools from main-def.cfg Loader `MaxThreads` (0: as many as the machine has). A
/// user-set count is used as is. With 0, the background pool that every loader shares (terrain,
/// stars, galaxy models, systems) and that Bevy builds its pipelines on gets a third of the
/// threads, at most 8, instead of Bevy's quarter capped at 4: the first frame's pipeline build
/// otherwise waits behind the galaxy model builds.
fn task_pool_options(max_threads: usize) -> TaskPoolOptions {
    match max_threads {
        0 => {
            let mut o = TaskPoolOptions::default();
            o.async_compute.percent = 1.0 / 3.0;
            o.async_compute.max_threads = 8;
            o
        }
        n => TaskPoolOptions::with_num_threads(n),
    }
}

/// The port draws no Bevy lights (its shaders never read the light clusters), so Bevy's GPU light
/// clustering, which runs every frame, is switched off (camera.rs gives the camera no clusters).
/// The resource is made in PbrPlugin's `finish`, so this runs at Startup.
fn no_light_clustering(settings: Option<ResMut<bevy::light::cluster::GlobalClusterSettings>>) {
    if let Some(mut s) = settings {
        s.gpu_clustering = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_max_threads_is_kept() {
        let o = task_pool_options(6);
        assert_eq!((o.min_total_threads, o.max_total_threads), (6, 6));
        // Only the default (0) widens the background pool.
        assert_eq!(o.async_compute.max_threads, TaskPoolOptions::default().async_compute.max_threads);
        let o = task_pool_options(0);
        assert_eq!((o.min_total_threads, o.max_total_threads), (1, usize::MAX));
        assert_eq!((o.async_compute.min_threads, o.async_compute.max_threads), (1, 8));
    }
}
