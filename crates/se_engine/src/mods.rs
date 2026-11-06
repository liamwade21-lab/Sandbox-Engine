//! The Mods menu (docs/spec-mods-viltrumite-powers.md, flow F1). Mods are built into the game, one
//! folder each under `assets/mods`, found once when the game starts: no plug-ins, nothing added without
//! a build. A folder's `mod.cfg` (SE's syntax, read with `se_core::sc`) names the mod, its `Code` (which
//! mod this build knows it as) and each file it needs (`Need "relative path"`, only what exists by now).
//!
//! [`ModRegistry`] is what was found, [`ModControl`] what is on. A row is grey, with the reason, when the
//! mod's files are missing or bad, when this build doesn't know its `Code`, while a ship is being piloted
//! and while a script runs; starting to pilot or a script while a mod is on turns it off first.
//!
//! With every mod off nothing here runs each frame but its run conditions (`SE_MODS=0` also hides the
//! button and skips the scan, for comparisons).

use std::path::{Component, PathBuf};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, PrimaryEguiContext, egui};

use crate::hud::HudState;
use crate::i18n::I18n;
use crate::scripting::ScriptRunner;
use crate::ship::Flight;

/// The `Code`s this build knows. A mod with any other came from a newer build: its row says so.
const KNOWN_CODES: &[&str] = &["viltrumite"];

/// `SE_MODS=0` (comparisons): no Mods button, no scan.
pub fn mods_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SE_MODS").map_or(true, |v| v != "0"))
}

/// Why a mod's row is grey.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GreyReason {
    /// A file the mod needs isn't there (the path as its `mod.cfg` gives it).
    Missing(String),
    /// A file the mod needs is there but can't be used: empty, unreadable, not a file, a path that
    /// leaves the mod's folder, or (from later steps) a parser's refusal.
    Bad(String),
    /// Its `Code` is unknown to this build.
    NewerBuild,
    Piloting,
    Script,
}

impl std::fmt::Display for GreyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GreyReason::Missing(p) => write!(f, "Missing {p}"),
            GreyReason::Bad(p) => write!(f, "Bad {p}"),
            GreyReason::NewerBuild => f.write_str("Needs a newer build"),
            GreyReason::Piloting => f.write_str("A ship is being piloted"),
            GreyReason::Script => f.write_str("A script is running"),
        }
    }
}

/// One folder under `assets/mods`.
#[derive(Clone, Debug)]
pub struct ModInfo {
    /// The folder's name.
    pub id: String,
    pub name: String,
    pub description: String,
    pub code: String,
    /// What keeps it off whatever the game is doing: a bad file or an unknown `Code`.
    pub fault: Option<GreyReason>,
}

/// The mods found on disk, scanned once at start.
#[derive(Resource, Default)]
pub struct ModRegistry {
    pub mods: Vec<ModInfo>,
}

impl ModRegistry {
    fn get(&self, id: &str) -> Option<&ModInfo> {
        self.mods.iter().find(|m| m.id == id)
    }

    /// A mod by its folder or its name, as a dev command gives it.
    fn find(&self, name: &str) -> Option<&ModInfo> {
        self.mods.iter().find(|m| m.id.eq_ignore_ascii_case(name) || m.name.eq_ignore_ascii_case(name))
    }

    fn name_of<'a>(&'a self, id: &'a str) -> &'a str {
        self.get(id).map_or(id, |m| &m.name)
    }

    /// A file turned out unusable when a mod loaded it (the parsers run on first enable): grey from now on.
    #[allow(dead_code)] // for the mod's loader, from step 3
    pub fn mark_bad(&mut self, id: &str, path: &str) {
        warn!("mods: {id} cannot be turned on: Bad {path}");
        if let Some(m) = self.mods.iter_mut().find(|m| m.id == id) {
            m.fault = Some(GreyReason::Bad(path.to_string()));
        }
    }
}

/// What the menu or a dev command asks for; [`apply_request`] is the one place that acts on it.
#[derive(Clone, Debug)]
enum ModRequest {
    On { id: String, dev: bool },
    Off,
}

/// What is on.
#[derive(Resource, Default)]
#[allow(dead_code)] // `menus` and `aftermath` are read from step 4 and step 8
pub struct ModControl {
    /// The folder name of the mod that is on. One at a time: a mod owns the camera and the keys.
    pub active: Option<String>,
    /// Tab's menus are open: the mouse goes to egui, the panels are back (step 4).
    pub menus: bool,
    /// A mod that was turned off still has pools or a broken table draining (steps 8 and 10).
    pub aftermath: bool,
    /// Turned on by a dev command: a test run with no real keyboard or mouse.
    pub dev: bool,
    /// Turned on and still loading its assets and pipelines: the row reads "Loading" (step 3).
    pub loading: bool,
    request: Option<ModRequest>,
}

impl ModControl {
    /// Turn the mod that is on off. Every way off comes through here.
    fn off(&mut self, reg: &ModRegistry, why: &str) {
        if let Some(id) = self.active.take() {
            info!("mods: {} off ({why})", reg.name_of(&id));
        }
        self.menus = false;
        self.dev = false;
        self.loading = false;
    }
}

/// The snapshot of the player's keys and mouse for the mod that is on: filled from step 4.
#[derive(Resource, Default)]
pub struct ModInput {}

pub struct ModsPlugin;

impl Plugin for ModsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModRegistry>()
            .init_resource::<ModControl>()
            .init_resource::<ModInput>()
            .add_systems(Startup, scan_at_start)
            // After whatever can start piloting this frame, and before a script's first command: the
            // mod goes off first.
            .add_systems(
                Update,
                (off_for_pilot_or_script.run_if(mod_on), apply_request.run_if(has_request))
                    .chain()
                    .after(crate::ship::ship_input)
                    .after(crate::camera::input_controls)
                    .before(crate::scripting::run_script),
            )
            // After bevy_window turns the closed window into AppExit, before the session is saved.
            .add_systems(Last, off_on_exit.after(bevy::window::ExitSystems).before(crate::session::save_on_exit).run_if(mod_on))
            .add_systems(EguiPrimaryContextPass, draw_mods_window.in_set(crate::skin::UiSet::Draw).run_if(window_open));
    }
}

fn mod_on(control: Res<ModControl>) -> bool {
    control.active.is_some()
}

fn has_request(control: Res<ModControl>) -> bool {
    control.request.is_some()
}

fn window_open(hud: Res<HudState>) -> bool {
    hud.show_mods && !hud.hidden && mods_enabled()
}

/// Where mods are looked for, in order: `asset_dir()` (the assets folder next to the exe when there is
/// one), then the crate's source tree, as orbital.rs's textures.
fn mod_roots() -> Vec<PathBuf> {
    let mut roots = vec![crate::config::asset_dir().join("mods")];
    let tree = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("mods");
    if !roots.contains(&tree) {
        roots.push(tree);
    }
    roots
}

/// A mod's file, by the path its `mod.cfg` gives, from the first root that has it.
#[allow(dead_code)] // for the mod's loader, from step 3
pub fn mod_file(id: &str, rel: &str) -> Option<PathBuf> {
    find_file(&mod_roots(), id, rel)
}

fn find_file(roots: &[PathBuf], id: &str, rel: &str) -> Option<PathBuf> {
    roots.iter().map(|r| r.join(id).join(rel)).find(|p| p.is_file())
}

/// Every folder in every root, the first root winning a name both have, sorted by name.
fn scan(roots: &[PathBuf]) -> Vec<ModInfo> {
    let mut ids: Vec<String> = roots
        .iter()
        .filter_map(|r| std::fs::read_dir(r).ok())
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids.dedup();
    ids.iter().map(|id| read_mod(roots, id)).collect()
}

/// A folder's `mod.cfg`, and the checks on what it needs. Whatever is wrong makes a grey row, never a panic.
fn read_mod(roots: &[PathBuf], id: &str) -> ModInfo {
    let mut info = ModInfo { id: id.to_string(), name: id.to_string(), description: String::new(), code: String::new(), fault: None };
    let Some(path) = find_file(roots, id, "mod.cfg") else {
        info.fault = Some(GreyReason::Missing("mod.cfg".into()));
        return info;
    };
    let bad_cfg = || Some(GreyReason::Bad("mod.cfg".into()));
    let Ok(text) = std::fs::read_to_string(&path) else {
        info.fault = bad_cfg();
        return info;
    };
    let cfg = se_core::sc::parse(&text);
    let (Some(name), Some(code)) = (cfg.str("Name"), cfg.str("Code")) else {
        info.fault = bad_cfg();
        return info;
    };
    info.name = name.to_string();
    info.code = code.to_string();
    info.description = cfg.str("Description").unwrap_or_default().to_string();
    if !KNOWN_CODES.contains(&code) {
        info.fault = Some(GreyReason::NewerBuild);
        return info;
    }
    info.fault = cfg.all("Need").find_map(|need| match need.str() {
        Some(rel) => check_need(roots, id, rel),
        None => bad_cfg(),
    });
    info
}

/// What's wrong with a file a mod needs, if anything. A path that leaves the mod's folder is bad, and an
/// empty file is: a copy cut short. The parsers look inside on first enable (`ModRegistry::mark_bad`).
fn check_need(roots: &[PathBuf], id: &str, rel: &str) -> Option<GreyReason> {
    let bad = || Some(GreyReason::Bad(rel.to_string()));
    let stays_inside = std::path::Path::new(rel).components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if rel.is_empty() || !stays_inside {
        return bad();
    }
    match find_file(roots, id, rel) {
        Some(p) => match std::fs::metadata(p) {
            Ok(m) if m.len() > 0 => None,
            _ => bad(),
        },
        None if roots.iter().any(|r| r.join(id).join(rel).exists()) => bad(),
        None => Some(GreyReason::Missing(rel.to_string())),
    }
}

/// Why this mod can't be turned on right now, if it can't. A mod that is on is never grey: it can always
/// be turned off.
pub fn grey_reason(info: &ModInfo, piloting: bool, script: bool) -> Option<GreyReason> {
    info.fault.clone().or_else(|| {
        if piloting {
            Some(GreyReason::Piloting)
        } else if script {
            Some(GreyReason::Script)
        } else {
            None
        }
    })
}

fn scan_at_start(mut reg: ResMut<ModRegistry>) {
    if !mods_enabled() {
        info!("mods: off (SE_MODS=0)");
        return;
    }
    reg.mods = scan(&mod_roots());
    for m in &reg.mods {
        match &m.fault {
            None => info!("mods: found {} ({})", m.name, m.id),
            Some(why) => warn!("mods: {} ({}) cannot be turned on: {why}", m.name, m.id),
        }
    }
}

/// Starting to pilot, or a script, turns the mod off first (flow F3).
fn off_for_pilot_or_script(mut control: ResMut<ModControl>, reg: Res<ModRegistry>, flight: Res<Flight>, runner: Res<ScriptRunner>) {
    if flight.active {
        control.off(&reg, "a ship is being piloted");
    } else if runner.starting() {
        control.off(&reg, "a script is starting");
    }
}

fn apply_request(mut control: ResMut<ModControl>, reg: Res<ModRegistry>, flight: Res<Flight>, runner: Res<ScriptRunner>) {
    match control.request.take() {
        Some(ModRequest::Off) => control.off(&reg, "asked"),
        Some(ModRequest::On { id, dev }) => {
            let Some(m) = reg.get(&id) else {
                warn!("mods: no mod {id}");
                return;
            };
            if control.active.as_deref() == Some(id.as_str()) {
                return;
            }
            if let Some(why) = grey_reason(m, flight.active, runner.starting()) {
                warn!("mods: {} stays off: {why}", m.name);
                return;
            }
            control.off(&reg, "another mod turned on");
            info!("mods: {} on", m.name);
            control.active = Some(id);
            control.dev = dev;
        }
        None => {}
    }
}

fn off_on_exit(mut exits: MessageReader<AppExit>, mut control: ResMut<ModControl>, reg: Res<ModRegistry>) {
    if exits.read().next().is_some() {
        control.off(&reg, "the game is closing");
    }
}

/// The window, a copy of Places: one selectable row per mod, its description under it.
fn draw_mods_window(
    mut contexts: EguiContexts,
    mut hud: ResMut<HudState>,
    reg: Res<ModRegistry>,
    mut control: ResMut<ModControl>,
    flight: Res<Flight>,
    runner: Res<ScriptRunner>,
    i18n: Res<I18n>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut open = true;
    let mut request = None;
    egui::Window::new(crate::skin::title(ctx, i18n.tr("Mods"))).open(&mut open).default_width(300.0).show(ctx, |ui| {
        if reg.mods.is_empty() {
            ui.weak(i18n.tr("No mods found"));
        }
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for m in &reg.mods {
                let on = control.active.as_deref() == Some(m.id.as_str());
                let grey = if on { None } else { grey_reason(m, flight.active, runner.starting()) };
                let state = match (on, control.loading) {
                    (true, true) => format!("  [{}]", i18n.tr("Loading")),
                    (true, false) => format!("  [{}]", i18n.tr("On")),
                    _ => String::new(),
                };
                let label = format!("{}{state}", i18n.tr(&m.name));
                let row = ui.add_enabled_ui(grey.is_none(), |ui| ui.selectable_label(on, label)).inner;
                let hint = if on { "Click to turn it off" } else { "Click to turn it on" };
                let row = row.on_hover_text(i18n.tr(hint));
                crate::script_gui::mark(ui, "Mods", &m.name, &row);
                if row.clicked() {
                    request = Some(if on { ModRequest::Off } else { ModRequest::On { id: m.id.clone(), dev: false } });
                }
                if !m.description.is_empty() {
                    ui.label(i18n.tr(&m.description)); // (weak text is nearly invisible under the skin)
                }
                if let Some(why) = grey {
                    ui.colored_label(ui.visuals().warn_fg_color, why.to_string());
                }
                ui.add_space(6.0);
            }
        });
    });
    hud.show_mods = open;
    if request.is_some() {
        control.request = request;
    }
    Ok(())
}

/// The dev command `mods list | on NAME | off | window [off]` (camera.rs's match).
pub fn dev_command(world: &mut World, arg: &str) {
    let (verb, name) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
    match verb.to_ascii_lowercase().as_str() {
        "list" => {
            let reg = world.resource::<ModRegistry>();
            let control = world.resource::<ModControl>();
            let piloting = world.resource::<Flight>().active;
            let script = world.resource::<ScriptRunner>().starting();
            info!("mods: {} found{}", reg.mods.len(), if mods_enabled() { "" } else { " (SE_MODS=0)" });
            for m in &reg.mods {
                let state = if control.active.as_deref() == Some(m.id.as_str()) {
                    "on".to_string()
                } else {
                    grey_reason(m, piloting, script).map_or("off".to_string(), |why| format!("grey: {why}"))
                };
                info!("mods: {} = {} (code {}): {state}", m.id, m.name, m.code);
            }
        }
        "on" => {
            let id = world.resource::<ModRegistry>().find(name).map(|m| m.id.clone());
            match id {
                Some(id) => world.resource_mut::<ModControl>().request = Some(ModRequest::On { id, dev: true }),
                None => warn!("mods: no mod named '{name}'"),
            }
        }
        "off" => world.resource_mut::<ModControl>().request = Some(ModRequest::Off),
        "window" => world.resource_mut::<HudState>().show_mods = name != "off",
        _ => warn!("mods: unknown command '{arg}'"),
    }
}

/// The dev command `uiclick X Y`: a left click for egui at that window point (its logical pixels), so a
/// test run, which has no mouse, can press real buttons.
pub fn dev_ui_click(world: &mut World, arg: &str) {
    let v: Vec<f32> = arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let [x, y] = v[..] else {
        warn!("uiclick: wants X Y, got '{arg}'");
        return;
    };
    let mut contexts = world.query_filtered::<&mut bevy_egui::EguiInput, With<PrimaryEguiContext>>();
    let Ok(mut input) = contexts.single_mut(world) else { return };
    let pos = egui::pos2(x, y);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    input.events.extend([egui::Event::PointerMoved(pos), button(true), button(false)]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// An empty folder of the test's own in the system's temp folder.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("se_mods_test_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &std::path::Path, id: &str, rel: &str, text: &str) {
        let path = root.join(id).join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    const GOOD: &str = "Name \"Test mod\"\nDescription \"For the tests\"\nCode \"viltrumite\"\nNeed \"a.bin\"\nNeed \"sub/b.bin\"\n";

    #[test]
    fn a_missing_file_greys_the_row_and_finding_it_clears_it() {
        let root = scratch("missing");
        write(&root, "m", "mod.cfg", GOOD);
        write(&root, "m", "a.bin", "x");
        let roots = [root.clone()];
        let m = read_mod(&roots, "m");
        assert_eq!((m.name.as_str(), m.description.as_str(), m.code.as_str()), ("Test mod", "For the tests", "viltrumite"));
        assert_eq!(m.fault, Some(GreyReason::Missing("sub/b.bin".into())));
        assert_eq!(m.fault.as_ref().unwrap().to_string(), "Missing sub/b.bin");
        write(&root, "m", "sub/b.bin", "x");
        assert_eq!(read_mod(&roots, "m").fault, None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_without_mod_cfg_is_a_grey_row_named_after_it() {
        let root = scratch("nocfg");
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join("a_file_not_a_mod"), "x").unwrap();
        let found = scan(&[root.clone()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "empty");
        assert_eq!(found[0].fault, Some(GreyReason::Missing("mod.cfg".into())));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bad_files_grey_the_row_and_nothing_panics() {
        let root = scratch("bad");
        let roots = [root.clone()];
        let cfg_bad = Some(GreyReason::Bad("mod.cfg".into()));
        // A mod.cfg that says nothing useful, or isn't text at all.
        for (id, text) in [("nothing", ""), ("no_code", "Name \"x\"\n"), ("no_name", "Code \"viltrumite\"\n"), ("garbage", "{{{ }} \" \u{1} ) Need"), ("need_number", "Name \"x\"\nCode \"viltrumite\"\nNeed 5\n")] {
            write(&root, id, "mod.cfg", text);
            assert_eq!(read_mod(&roots, id).fault, cfg_bad, "{id}");
        }
        fs::create_dir_all(root.join("binary")).unwrap();
        fs::write(root.join("binary").join("mod.cfg"), [0xffu8, 0xfe, 0x00, 0x80]).unwrap();
        assert_eq!(read_mod(&roots, "binary").fault, cfg_bad);
        // A needed file cut short, a folder where a file should be, a path that leaves the mod's folder.
        write(&root, "m", "empty.bin", "");
        fs::create_dir_all(root.join("m").join("dir.bin")).unwrap();
        write(&root, "m", "mod.cfg", "Name \"x\"\nCode \"viltrumite\"\nNeed \"empty.bin\"\n");
        assert_eq!(read_mod(&roots, "m").fault, Some(GreyReason::Bad("empty.bin".into())));
        write(&root, "m", "mod.cfg", "Name \"x\"\nCode \"viltrumite\"\nNeed \"dir.bin\"\n");
        assert_eq!(read_mod(&roots, "m").fault, Some(GreyReason::Bad("dir.bin".into())));
        for rel in ["../other/mod.cfg", "C:/Windows/notepad.exe", "/etc/passwd", ""] {
            write(&root, "m", "mod.cfg", &format!("Name \"x\"\nCode \"viltrumite\"\nNeed \"{rel}\"\n"));
            assert_eq!(read_mod(&roots, "m").fault, Some(GreyReason::Bad(rel.into())), "{rel}");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_unknown_code_needs_a_newer_build() {
        let root = scratch("code");
        write(&root, "m", "mod.cfg", "Name \"From the future\"\nCode \"warp_kittens\"\nNeed \"never_there.bin\"\n");
        let m = read_mod(&[root.clone()], "m");
        assert_eq!(m.name, "From the future");
        assert_eq!(m.fault, Some(GreyReason::NewerBuild));
        assert_eq!(m.fault.unwrap().to_string(), "Needs a newer build");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_first_root_wins_and_the_second_fills_in() {
        let (a, b) = (scratch("root_a"), scratch("root_b"));
        // Beside the exe: its own copy of a mod, whose file is only in the source tree (orbital.rs's rule
        // is per file). Only in the source tree: another mod.
        write(&a, "both", "mod.cfg", "Name \"From A\"\nCode \"viltrumite\"\nNeed \"only_b.bin\"\n");
        write(&b, "both", "mod.cfg", "Name \"From B\"\nCode \"viltrumite\"\n");
        write(&b, "both", "only_b.bin", "x");
        write(&b, "only_b", "mod.cfg", "Name \"Tree\"\nCode \"viltrumite\"\n");
        // No `mods` folder at all beside the exe is the usual case (the baseline's folder, the run from target).
        let roots = [a.join("no_such_folder"), a.clone(), b.clone()];
        let found = scan(&roots);
        assert_eq!(found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["both", "only_b"]);
        assert_eq!(found[0].name, "From A");
        assert_eq!(found[0].fault, None);
        assert_eq!(found[1].name, "Tree");
        assert_eq!(find_file(&roots, "both", "only_b.bin"), Some(b.join("both").join("only_b.bin")));
        assert_eq!(find_file(&roots, "both", "mod.cfg"), Some(a.join("both").join("mod.cfg")));
        assert_eq!(scan(&[a.join("no_such_folder")]).len(), 0);
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn the_shipped_mod_is_good_in_this_tree() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("mods");
        let found = scan(&[root]);
        let m = found.iter().find(|m| m.id == "viltrumite").expect("assets/mods/viltrumite");
        assert_eq!(m.code, "viltrumite");
        assert_eq!(m.name, "Viltrumite powers");
        assert_eq!(m.fault, None);
        assert!(!m.description.is_empty());
    }

    #[test]
    fn a_file_fault_comes_before_what_the_game_is_doing() {
        let mut m = ModInfo { id: "m".into(), name: "M".into(), description: String::new(), code: "viltrumite".into(), fault: None };
        assert_eq!(grey_reason(&m, false, false), None);
        assert_eq!(grey_reason(&m, true, false), Some(GreyReason::Piloting));
        assert_eq!(grey_reason(&m, false, true), Some(GreyReason::Script));
        assert_eq!(grey_reason(&m, true, true), Some(GreyReason::Piloting));
        m.fault = Some(GreyReason::Missing("a".into()));
        assert_eq!(grey_reason(&m, true, true), m.fault);
    }

    #[test]
    fn a_parser_refusing_a_file_greys_the_row_from_then_on() {
        let mut reg = ModRegistry {
            mods: vec![ModInfo { id: "m".into(), name: "M".into(), description: String::new(), code: "viltrumite".into(), fault: None }],
        };
        reg.mark_bad("m", "omniman.sevm");
        reg.mark_bad("nobody", "x");
        assert_eq!(grey_reason(reg.get("m").unwrap(), false, false), Some(GreyReason::Bad("omniman.sevm".into())));
    }

    /// The two systems that decide what is on, without the rest of the game.
    fn control_app(mods: &[(&str, Option<GreyReason>)]) -> App {
        let mut app = App::new();
        let mods = mods
            .iter()
            .map(|(id, fault)| ModInfo { id: id.to_string(), name: id.to_string(), description: String::new(), code: "viltrumite".into(), fault: fault.clone() })
            .collect();
        app.insert_resource(ModRegistry { mods })
            .init_resource::<ModControl>()
            .init_resource::<Flight>()
            .init_resource::<ScriptRunner>()
            .add_systems(Update, (off_for_pilot_or_script.run_if(mod_on), apply_request.run_if(has_request)).chain());
        app
    }

    fn ask(app: &mut App, request: ModRequest) {
        app.world_mut().resource_mut::<ModControl>().request = Some(request);
        app.update();
    }

    fn on(id: &str) -> ModRequest {
        ModRequest::On { id: id.to_string(), dev: false }
    }

    fn active(app: &App) -> Option<String> {
        app.world().resource::<ModControl>().active.clone()
    }

    #[test]
    fn turning_on_and_off() {
        let mut app = control_app(&[("a", None), ("b", None), ("grey", Some(GreyReason::Missing("x".into())))]);
        ask(&mut app, on("a"));
        assert_eq!(active(&app).as_deref(), Some("a"));
        // One at a time; a grey or unknown mod changes nothing; off is always allowed.
        ask(&mut app, on("b"));
        assert_eq!(active(&app).as_deref(), Some("b"));
        ask(&mut app, on("grey"));
        ask(&mut app, on("nobody"));
        assert_eq!(active(&app).as_deref(), Some("b"));
        ask(&mut app, ModRequest::Off);
        assert_eq!(active(&app), None);
        ask(&mut app, ModRequest::Off);
        assert_eq!(active(&app), None);
    }

    #[test]
    fn piloting_turns_it_off_and_keeps_it_off() {
        let mut app = control_app(&[("a", None)]);
        ask(&mut app, on("a"));
        app.world_mut().resource_mut::<Flight>().active = true;
        app.update();
        assert_eq!(active(&app), None);
        ask(&mut app, on("a"));
        assert_eq!(active(&app), None);
        app.world_mut().resource_mut::<Flight>().active = false;
        ask(&mut app, on("a"));
        assert_eq!(active(&app).as_deref(), Some("a"));
    }

    #[test]
    fn a_script_starting_turns_it_off_and_keeps_it_off() {
        let mut app = control_app(&[("a", None)]);
        ask(&mut app, on("a"));
        app.world_mut().resource_mut::<ScriptRunner>().request("scripts/x.se");
        app.update();
        assert_eq!(active(&app), None);
        ask(&mut app, on("a"));
        assert_eq!(active(&app), None);
    }
}
