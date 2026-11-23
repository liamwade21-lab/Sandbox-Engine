//! Making the Faulcon DeLacy Sidewinder mod's files (stage 2 of ARCHITECTURE.md), once:
//! `cargo test -p se_engine --release copy_sidewinder -- --ignored --nocapture`. Only this test and the ignored checks
//! below read Elite's install or the decoded export; the game never does (the spec's C4).
//!
//! Inputs: `ELITE_PATH`, the install's product folder (default: the user's Steam copy), read only, for the key files
//! (`ControlSchemes`) and the version line of `SOURCES.txt` (`VersionInfo.txt`) alone; `ELITE_EXPORT`, the Sidewinder's
//! export (default `re/elite/frea_export/sidewinder`; its `decoded/` subfolder is what is read, `README.md` there the
//! reference) for Elite's original assets; and the audio export beside it (`sidewinder_audio`). Without the decoded
//! export the run fails, naming it. The ship UI's export beside it (`sidewinder_ui`: its `audio/` and `decoded/sounds/`)
//! gives the UI's sounds; its decoded folder (`ELITE_UI_EXPORT`, default `sidewinder_ui/decoded` beside `ELITE_EXPORT`)
//! the UI's movies, fonts, strings, layout and panel models. The FREA key is never read.
//!
//! Each kind of file is its own file here, `convert(ctx, shipped)` writing its files into `Ctx::out`, reading them back
//! and reporting each in `Shipped`: `binds.rs` (Elite's key files), `models.rs` (Elite's models, materials and
//! textures), `assembly.rs` (`ship.json`, checked against the models by `assembly::check`), `chair.rs`, `sound.rs` (the
//! ship's and the UI's events in one `sounds.json`), `ui_sound.rs` (which UI events, and `ui/sound_tables.json`),
//! `ui_movies.rs` (the UI's movies, fonts, strings, focus machine and controls under `ui/`), `ui_layout.rs` (`ui/layout.json`
//! with the panel models and their textures, which `ui_models.rs` makes).
//! Then `SOURCES.txt` and `mod.cfg` are written from `Shipped`, and every file of the folder is read back: each must be
//! a `Need`, read as the game reads it, and be at most 90 MB. A file the converter no longer ships is not deleted by it:
//! the read-back refuses the folder until it is removed (`git rm`).

mod assembly;
mod binds;
mod chair;
mod models;
mod sound;
mod ui_layout;
mod ui_models;
mod ui_movies;
mod ui_sound;

use std::path::{Path, PathBuf};

/// Files over this are refused (GitHub refuses 100 MB; the project's rule is 90).
const MAX_FILE_BYTES: u64 = 90 << 20;

fn install() -> PathBuf {
    std::env::var_os("ELITE_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"D:\SteamLibrary\steamapps\common\Elite Dangerous\Products\elite-dangerous-odyssey-64"))
}

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("mods").join("sidewinder")
}

/// Where the converter reads and writes.
pub(super) struct Ctx {
    /// Elite's install, the product folder (read only).
    pub install: PathBuf,
    /// The decoded export (`ELITE_EXPORT`'s `decoded/`).
    pub export: PathBuf,
    /// The audio export (`sidewinder_audio`, beside `ELITE_EXPORT`): `sound.rs`'s.
    pub audio: PathBuf,
    /// The ship UI's export (`sidewinder_ui`, beside `ELITE_EXPORT`): its `audio/` and `decoded/sounds/`, `ui_sound.rs`'s.
    pub ui: PathBuf,
    /// The ship UI's decoded export (`ELITE_UI_EXPORT`, default `ui`'s `decoded/`; its `README.md` the handoff): the
    /// `ui_movies.rs`, `ui_layout.rs` and `ui_models.rs` input.
    pub ui_decoded: PathBuf,
    /// The mod's folder.
    pub out: PathBuf,
}

impl Ctx {
    fn from_env() -> Result<Ctx, String> {
        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let export_root = std::env::var_os("ELITE_EXPORT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("re/elite/frea_export/sidewinder"));
        let export_root = if export_root.is_absolute() { export_root } else { project.join(export_root) };
        let export = export_root.join("decoded");
        if !export.join("manifest.json").is_file() {
            return Err(format!(
                "ELITE_EXPORT: no decoded export at {} (its manifest.json is missing). Make it with re/elite/tools/sidewinder_decode.py, \
                 or set ELITE_EXPORT to the folder holding decoded/",
                export.display()
            ));
        }
        let audio = export_root.parent().map(|p| p.join("sidewinder_audio")).unwrap_or_default();
        let ui = export_root.parent().map(|p| p.join("sidewinder_ui")).unwrap_or_default();
        let ui_decoded = std::env::var_os("ELITE_UI_EXPORT").map(PathBuf::from).unwrap_or_else(|| ui.join("decoded"));
        let ui_decoded = if ui_decoded.is_absolute() { ui_decoded } else { project.join(ui_decoded) };
        Ok(Ctx { install: install(), export, audio, ui, ui_decoded, out: out_dir() })
    }

    /// Writes a file of the mod's folder (`rel` with forward slashes), making its folders.
    pub fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), String> {
        let p = self.out.join(rel);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&p, bytes).map_err(|e| format!("{rel}: {e}"))
    }

    /// Reads a file of the mod's folder back.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>, String> {
        std::fs::read(self.out.join(rel)).map_err(|e| format!("{rel}: {e}"))
    }
}

/// One shipped file: what `SOURCES.txt` says of it and what `mod.cfg` needs.
pub(super) struct ShippedFile {
    /// Relative to the mod's folder, forward slashes.
    pub path: String,
    /// Where it came from: export files, Elite's own file names.
    pub sources: Vec<String>,
    /// The spec's grade (A = Elite's own bytes ... E = recreated).
    pub grade: &'static str,
    pub note: String,
}

/// A file a kind plans to ship, worked out from the export alone: written by `Planned::ship`, compared with the folder by
/// `Planned::same` (the UI's kinds plan first, then write or verify).
pub(super) struct Planned {
    /// Relative to the mod's folder, forward slashes.
    pub path: String,
    pub bytes: Vec<u8>,
    pub sources: Vec<String>,
    pub grade: &'static str,
    pub note: String,
}

impl Planned {
    /// Writes each file, reads it back the same, and reports it in `Shipped`.
    pub fn ship(ctx: &Ctx, shipped: &mut Shipped, files: Vec<Planned>) -> Result<(), String> {
        for f in files {
            ctx.write(&f.path, &f.bytes)?;
            if ctx.read(&f.path)? != f.bytes {
                return Err(format!("{} reads back different", f.path));
            }
            shipped.files.push(ShippedFile { path: f.path, sources: f.sources, grade: f.grade, note: f.note });
        }
        Ok(())
    }

    /// The folder holds each file byte for byte, and `dirs` (folders only these kinds write) hold nothing else.
    pub fn same(ctx: &Ctx, files: &[Planned], dirs: &[&str]) -> Result<(), String> {
        for f in files {
            if ctx.read(&f.path)? != f.bytes {
                return Err(format!("{} differs from what the export makes", f.path));
            }
        }
        for dir in dirs {
            let mut held = Vec::new();
            folder_files(&ctx.out.join(dir), &format!("{dir}/"), &mut held)?;
            let mut held: Vec<String> = held.into_iter().map(|(n, _)| n).collect();
            let mut want: Vec<String> = files.iter().map(|f| f.path.clone()).filter(|p| p.starts_with(&format!("{dir}/"))).collect();
            held.sort();
            want.sort();
            if held != want {
                return Err(format!("{dir}/ holds {} files, the export makes {}: {:?}", held.len(), want.len(), held.iter().filter(|h| !want.contains(h)).collect::<Vec<_>>()));
            }
        }
        Ok(())
    }
}

/// Every file the kinds wrote, in order, and their lines on what is left out on purpose.
#[derive(Default)]
pub(super) struct Shipped {
    pub files: Vec<ShippedFile>,
    pub notes: Vec<String>,
}

impl Shipped {
    pub fn add(&mut self, path: &str, sources: &[String], grade: &'static str, note: &str) {
        self.files.push(ShippedFile { path: path.to_string(), sources: sources.to_vec(), grade, note: note.to_string() });
    }

    /// A line for `SOURCES.txt` on something not shipped.
    pub fn note(&mut self, line: &str) {
        self.notes.push(line.to_string());
    }
}

#[test]
#[ignore]
fn copy_sidewinder() {
    if let Err(e) = run() {
        panic!("copy_sidewinder: {e}");
    }
}

fn run() -> Result<(), String> {
    let ctx = Ctx::from_env()?;
    let mut shipped = Shipped::default();
    binds::convert(&ctx, &mut shipped)?;
    models::convert(&ctx, &mut shipped)?;
    assembly::convert(&ctx, &mut shipped)?;
    chair::convert(&ctx, &mut shipped)?;
    sound::convert(&ctx, &mut shipped)?;
    ui_sound::convert(&ctx, &mut shipped)?;
    ui_movies::convert(&ctx, &mut shipped)?;
    ui_layout::convert(&ctx, &mut shipped)?;
    write_sources(&ctx, &mut shipped)?;
    write_cfg(&ctx, &shipped)?;
    read_back(&ctx, &shipped)
}

/// `SOURCES.txt`: each shipped file with its sources and grade.
fn write_sources(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    // (VersionInfo.txt is a line of JSON: only its "Version" is wanted.)
    let info = std::fs::read_to_string(ctx.install.join("VersionInfo.txt")).unwrap_or_default();
    let version = info.split("\"Version\"").nth(1).and_then(|s| s.split('"').nth(1)).unwrap_or("").trim().to_string();
    shipped.add("SOURCES.txt", &[], "-", "This list.");
    let mut text = format!(
        "The Faulcon DeLacy Sidewinder mod's files: where each came from. Written by copy_sidewinder (src/sidewinder/convert/).\n\
         Elite Dangerous is Frontier Developments'; every file below taken from it is theirs (install {}).\n\
         Grades are the plan's (docs/spec-mods-sidewinder.md): A = Elite's own bytes, E = recreated.\n\
         Elite's original assets come from the decoded export (re/elite/frea_export/sidewinder/decoded, made from the install by \
         the export tool, out of git): its README gives the exact vertex decode and the spaces.\n\
         Models: LOD0 only: Elite's LOD switch rule is not known.\n\n",
        if version.is_empty() { "version unknown".to_string() } else { format!("version {version}") },
    );
    for n in &shipped.notes {
        text.push_str(&format!("{n}\n\n"));
    }
    for f in &shipped.files {
        text.push_str(&format!("{} ({}): {}\n", f.path, f.grade, f.note));
        if !f.sources.is_empty() {
            text.push_str(&format!("  from: {}\n", f.sources.join("; ")));
        }
    }
    ctx.write("SOURCES.txt", text.as_bytes())
}

/// `mod.cfg`: the Mods menu's entry, a `Need` for every shipped file.
fn write_cfg(ctx: &Ctx, shipped: &Shipped) -> Result<(), String> {
    let mut files: Vec<&str> = shipped.files.iter().map(|f| f.path.as_str()).collect();
    files.sort();
    if let Some(w) = files.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!("{} is shipped twice", w[0]));
    }
    let mut cfg = String::from(
        "// The Mods menu reads this at start (mods.rs). Written by copy_sidewinder (sidewinder/convert/): every file the mod needs.\n\
         Name \"Faulcon DeLacy Sidewinder\"\n\
         Description \"Elite Dangerous's starter ship: spawn it where you look, walk up and press E to fly it by Elite's rules.\"\n\
         Code \"sidewinder\"\n",
    );
    for f in files {
        cfg.push_str(&format!("Need \"{f}\"\n"));
    }
    ctx.write("mod.cfg", cfg.as_bytes())
}

/// Every file in the folder (forward slashes), but `mod.cfg`.
fn folder_files(dir: &Path, rel: &str, out: &mut Vec<(String, u64)>) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = format!("{rel}{}", e.file_name().to_string_lossy());
        let meta = e.metadata().map_err(|e| format!("{name}: {e}"))?;
        if meta.is_dir() {
            folder_files(&e.path(), &format!("{name}/"), out)?;
        } else if name != "mod.cfg" {
            out.push((name, meta.len()));
        }
    }
    Ok(())
}

/// The folder holds exactly the `Need`s, each read as the game reads it, none over 90 MB, one for each file shipped.
fn read_back(ctx: &Ctx, shipped: &Shipped) -> Result<(), String> {
    let needs = check_folder(ctx)?;
    if needs != shipped.files.len() {
        return Err(format!("{needs} Needs for {} shipped files", shipped.files.len()));
    }
    Ok(())
}

/// The folder holds exactly `mod.cfg`'s `Need`s, each read as the game reads it, none over 90 MB. Their count.
fn check_folder(ctx: &Ctx) -> Result<usize, String> {
    let cfg = String::from_utf8(ctx.read("mod.cfg")?).map_err(|_| "mod.cfg is not text".to_string())?;
    let mut needs: Vec<String> = cfg.lines().filter_map(|l| l.strip_prefix("Need \"")?.strip_suffix('"').map(str::to_string)).collect();
    needs.sort();
    let mut files = Vec::new();
    folder_files(&ctx.out, "", &mut files)?;
    files.sort();
    let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
    if names != needs {
        let extra: Vec<_> = names.iter().filter(|n| !needs.contains(n)).collect();
        let missing: Vec<_> = needs.iter().filter(|n| !names.contains(n)).collect();
        return Err(format!("mod.cfg and the folder differ: not needed {extra:?}, missing {missing:?}"));
    }
    for (name, size) in &files {
        if *size > MAX_FILE_BYTES {
            return Err(format!("{name} is {size} bytes, over 90 MB"));
        }
        let bytes = ctx.read(name)?;
        let checked = match Path::new(name).extension().and_then(|e| e.to_str()) {
            Some("edm") => se_core::elite_asset::parse_edm(&bytes).map(|_| ()).map_err(|e| e.to_string()),
            Some("dds") => se_core::dds::parse_dds(&bytes).map(|_| ()).map_err(|e| e.to_string()),
            Some("binds") => se_core::elite_asset::Binds::parse(&String::from_utf8_lossy(&bytes)).map(|_| ()).map_err(|e| e.to_string()),
            Some("json") if name == "materials.json" => se_core::elite_ship::parse_materials(&bytes).map(|_| ()).map_err(|e| e.to_string()),
            Some("json") if name == "ship.json" => assembly::check(ctx, &bytes),
            Some("json") if name == "sounds.json" => sound::check(&bytes),
            Some("json") if name == "ui/sound_tables.json" => ui_sound::check_tables(&bytes),
            Some("json") if name == ui_layout::LAYOUT => ui_layout::check(ctx, &bytes),
            Some("json") if name == ui_movies::FONTMAP => ui_movies::check_fontmap(ctx, &bytes),
            Some("json") if name == ui_movies::STRINGS => ui_movies::check_strings(&bytes),
            Some("json") if name == ui_movies::FOCUS => ui_movies::check_focus(&bytes),
            Some("swf") => ui_movies::check_movie(&bytes),
            Some("ttf") => ui_movies::check_font(&bytes),
            Some("json") => serde_json::from_slice::<serde_json::Value>(&bytes).map(|_| ()).map_err(|e| e.to_string()),
            _ => Ok(()),
        };
        checked.map_err(|e| format!("{name}: {e}"))?;
    }
    let total: u64 = files.iter().map(|(_, s)| s).sum();
    let largest = files.iter().max_by_key(|(_, s)| *s).map(|(n, s)| format!("{n} ({:.1} MB)", *s as f64 / 1048576.0)).unwrap_or_default();
    println!("sidewinder: {} files, {:.1} MB in all, the largest {largest}", files.len(), total as f64 / 1048576.0);
    Ok(needs.len())
}

/// Re-checks the shipped folder against the export without converting (writes nothing): every file a `Need` and read
/// as the game reads it, the models' draw bytes and bones, the materials and every texture byte for byte as a fresh
/// plan from the export makes them, `ship.json` as the decoded files give it (with its checks against the models), and the
/// UI's movies, fonts, strings, focus data, layout, panel models and textures as the UI export makes them.
#[test]
#[ignore]
fn shipped_files_are_the_exports() {
    let check = || -> Result<(), String> {
        let ctx = Ctx::from_env()?;
        check_folder(&ctx)?;
        models::verify(&ctx)?;
        assembly::verify(&ctx)?;
        chair::verify(&ctx)?;
        ui_movies::verify(&ctx)?;
        ui_layout::verify(&ctx)
    };
    if let Err(e) = check() {
        panic!("shipped_files_are_the_exports: {e}");
    }
}

/// C12: the shipped key files are Elite's, byte for byte.
#[test]
#[ignore]
fn bindings_match_elite() {
    for b in binds::FILES {
        let ours = std::fs::read(out_dir().join("binds").join(b)).unwrap();
        let theirs = std::fs::read(install().join("ControlSchemes").join(b)).unwrap();
        assert_eq!(ours, theirs, "{b}");
    }
}
