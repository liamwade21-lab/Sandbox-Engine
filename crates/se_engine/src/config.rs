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
            mr_path: std::env::var_os("MR_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(se_core::mr::DEFAULT_MR_PATH)),
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

/// Directory holding the port's own assets (shaders). Prefers `assets/` next to
/// the executable, falling back to the crate's source tree during development.
pub fn asset_dir() -> PathBuf {
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
