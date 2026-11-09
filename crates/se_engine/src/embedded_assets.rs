//! The port's assets built into the exe (`build.rs`: shaders, textures, models, `mr`, `mods`). A player's copy
//! has no `assets` folder beside it, so on first start (and after an update changes them) they are
//! unpacked to `%LOCALAPPDATA%\SpaceEngine-rs-v2\assets` and read from there like any assets folder.
//! An exe inside the project folder reads the source tree instead (`config::source_assets`), so
//! shader edits still show without a rebuild.

use std::path::PathBuf;

include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

const STAMP_FILE: &str = "embedded.stamp";

/// Where the built-in assets go: `%LOCALAPPDATA%\SpaceEngine-rs-v2\assets`.
pub fn target_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("APPDATA"))?;
    Some(PathBuf::from(base).join(crate::settings::USER_DIR).join("assets"))
}

/// The folder the built-in assets are unpacked to (unpacking them if it isn't up to date), or
/// `None` if that folder can't be written.
pub fn unpacked() -> Option<PathBuf> {
    let dir = target_dir()?;
    let stamp = dir.join(STAMP_FILE);
    if std::fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == STAMP) && dir.join("shaders").is_dir() {
        return Some(dir);
    }
    for (rel, bytes) in FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        if std::fs::read(&path).ok().as_deref() != Some(*bytes) {
            std::fs::write(&path, bytes).ok()?;
        }
    }
    // Last, so an interrupted unpack is redone next start.
    std::fs::write(&stamp, STAMP).ok()?;
    eprintln!("Unpacked {} built-in asset files to {}", FILES.len(), dir.display());
    Some(dir)
}
