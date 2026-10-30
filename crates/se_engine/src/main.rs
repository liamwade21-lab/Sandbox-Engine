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
mod music;
mod overlay;
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
mod waypoints;
mod wiki;

use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy_egui::EguiPlugin;

fn main() {
    let cfg = config::EngineConfig::from_args();
    let asset_dir = config::asset_dir();
    let display = display::startup_settings(&cfg);

    let mut app = App::new();
    // Before the plugins: the log file reads SE's Log settings from the install.
    app.insert_resource(cfg.clone());
    // main-def.cfg Loader `MaxThreads` (0: as many as the machine has).
    let task_pools = match stars::main_def_f64(&cfg, "MaxThreads").map_or(0, |v| v.max(0.0) as usize) {
        0 => TaskPoolOptions::default(),
        n => TaskPoolOptions::with_num_threads(n),
    };
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
    .add_plugins((data::DataPlugin, universe::UniversePlugin, camera::CameraPlugin, memory::MemoryPlugin, exposure::ExposurePlugin, color_correction::ColorCorrectionPlugin, render_scale::RenderScalePlugin))
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
    .add_plugins((capture::CapturePlugin, projection::ProjectionPlugin));
    app.run();
}
