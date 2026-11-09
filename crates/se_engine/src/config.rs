//! Command line / environment configuration.

use bevy::prelude::*;
use std::path::PathBuf;

const DEFAULT_SE_PATH: &str = "D:/SteamLibrary/steamapps/common/SpaceEngine";

#[derive(Resource, Debug, Clone)]
pub struct EngineConfig {
    /// Root of the SpaceEngine installation (contains `data/`).
    pub se_path: PathBuf,
    /// `SE_PATH` or `--se-path` named the install (it must still pass `ownership::check`).
    pub se_path_given: bool,
    /// Root of the user's Megaton Rainfall install (contains `Assets/`), whose trees, buildings
    /// and cars the vegetation and cities read (`mr_assets.rs`). `--mr-path` or `MR_PATH`.
    pub mr_path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Object to start at (catalog name).
    pub start_at: String,
    /// `--start` was given: the saved session (`session.rs`) is not restored.
    pub start_given: bool,
    /// Save a screenshot to this path after `screenshot_frames` frames, then exit.
    pub screenshot: Option<PathBuf>,
    pub screenshot_frames: u32,
    /// Script of commands to run at startup (semicolon separated, see `camera::Command`).
    pub commands: Vec<String>,
    /// SpaceEngine `.se` script to run at startup (VFS path or file path).
    pub script: Option<String>,
    /// Let scripts continue on their own when they wait for the user (testing).
    pub script_auto: bool,
}

impl EngineConfig {
    pub fn from_args() -> Self {
        let mut cfg = EngineConfig {
            se_path: std::env::var_os("SE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SE_PATH)),
            se_path_given: std::env::var_os("SE_PATH").is_some(),
            // The default (the user's own install) only for an exe inside the project folder: a
            // player's copy reads Megaton Rainfall only when told where (`--mr-path`, `MR_PATH`).
            mr_path: std::env::var_os("MR_PATH")
                .map(PathBuf::from)
                .or_else(|| source_assets().map(|_| PathBuf::from(se_core::mr::DEFAULT_MR_PATH)))
                .unwrap_or_default(),
            width: 1600,
            height: 900,
            start_at: "Earth".into(),
            start_given: false,
            screenshot: None,
            screenshot_frames: 120,
            commands: Vec::new(),
            script: None,
            script_auto: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--se-path" => {
                    if let Some(p) = args.next() {
                        cfg.se_path = PathBuf::from(p);
                        cfg.se_path_given = true;
                    }
                }
                "--mr-path" => cfg.mr_path = args.next().map(PathBuf::from).unwrap_or(cfg.mr_path),
                "--size" => {
                    if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                        cfg.width = w.parse().unwrap_or(cfg.width);
                        cfg.height = h.parse().unwrap_or(cfg.height);
                    }
                }
                "--start" => {
                    cfg.start_at = args.next().unwrap_or(cfg.start_at);
                    cfg.start_given = true;
                }
                "--screenshot" => cfg.screenshot = args.next().map(PathBuf::from),
                "--frames" => cfg.screenshot_frames = args.next().and_then(|s| s.parse().ok()).unwrap_or(120),
                "--run" => {
                    if let Some(s) = args.next() {
                        cfg.commands.extend(s.split(';').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()));
                    }
                }
                "--script" => cfg.script = args.next(),
                "--script-auto" => cfg.script_auto = true,
                "--help" | "-h" => {
                    println!(
                        "spaceengine-rs [--se-path DIR] [--mr-path DIR] [--size WxH] [--start NAME] \
                         [--run \"cmd; cmd\"] [--script PATH.se [--script-auto]] [--screenshot FILE.png --frames N]"
                    );
                    std::process::exit(0);
                }
                other => eprintln!("ignoring unknown argument {other}"),
            }
        }
        cfg
    }
}

/// The `assets` folder next to the executable (what a player's copy ships with).
fn exe_assets() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("assets"))
}

/// The crate's own assets in the source tree, only for an exe that runs from inside the project
/// folder (`target\release`, test builds, bench copies in `.optimize/scratch`). A copy outside it,
/// what players get, never reads the source tree, even on the PC that built it.
pub fn source_assets() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project = std::fs::canonicalize(manifest.join("..").join("..")).ok()?;
    let exe = std::fs::canonicalize(std::env::current_exe().ok()?).ok()?;
    let tree = manifest.join("assets");
    (exe.starts_with(&project) && tree.join("shaders").is_dir()).then_some(tree)
}

/// Directory holding the port's own assets (shaders, textures, models), found once: `assets/` next
/// to the executable, then the source tree for an exe inside the project (`source_assets`), then the
/// copy built into the exe, unpacked (`embedded_assets`: what a player's copy uses).
pub fn asset_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let beside = exe_assets();
        if let Some(p) = beside.as_ref().filter(|p| p.join("shaders").is_dir()) {
            return p.clone();
        }
        source_assets()
            .or_else(crate::embedded_assets::unpacked)
            .or(beside)
            .unwrap_or_else(|| PathBuf::from("assets"))
    })
    .clone()
}

/// Whether the port's shaders were found (without them the universe draws black).
pub fn assets_found() -> bool {
    asset_dir().join("shaders").is_dir()
}

/// Every assets folder to look in for one file, in order: `asset_dir()`, then the source tree's
/// (an exe inside the project with only its shaders beside it).
pub fn asset_roots() -> Vec<PathBuf> {
    let mut roots = vec![asset_dir()];
    if let Some(tree) = source_assets()
        && !roots.contains(&tree)
    {
        roots.push(tree);
    }
    roots
}

/// One of the port's own files, `rel` inside an assets folder, from the first root that has it.
pub fn asset_file(rel: &std::path::Path) -> Option<PathBuf> {
    asset_roots().into_iter().map(|r| r.join(rel)).find(|p| p.is_file())
}
