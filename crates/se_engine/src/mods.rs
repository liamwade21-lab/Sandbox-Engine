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

use bevy::audio::{Decodable, Source, Volume};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, PrimaryEguiContext, egui};
use se_core::hero_asset::{self, ClipSet, HeroModel};

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
    /// leaves the mod's folder, or a refusal when first enable looks inside it.
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
    /// The `Need` lines of its `mod.cfg`. The scan only looks at their sizes; first enable looks inside.
    pub needs: Vec<String>,
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

    /// A file turned out unusable when a mod loaded it (the checks and parsers run on first enable): grey from now on.
    pub fn mark(&mut self, id: &str, why: GreyReason) {
        warn!("mods: {id} cannot be turned on: {why}");
        if let Some(m) = self.mods.iter_mut().find(|m| m.id == id) {
            m.fault = Some(why);
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
    /// Turned on and still loading its assets and pipelines: the row reads "Loading". `viltrumite::draw`
    /// clears it when he can show.
    pub loading: bool,
    /// The mod that is on is Viltrumite powers (`Code "viltrumite"`): what the hero's systems run on.
    pub hero: bool,
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
        self.hero = false;
    }
}

/// The Viltrumite mod's parsed model and clips, from its first enable on (spec, F1: loaded once, kept
/// until quit). Parsing is the check on them: a file the parsers refuse greys the row.
#[derive(Resource)]
pub struct HeroAssets {
    pub model: HeroModel,
    pub clips: ClipSet,
}

/// What greys a row at first enable, and what to log about it.
type FileFault = (GreyReason, String);

/// A file a mod needs, read now: `Missing` if it has gone since the scan, `Bad` if it can't be read.
fn read_need(roots: &[PathBuf], id: &str, rel: &str) -> Result<Vec<u8>, FileFault> {
    if let Some(why) = check_need(roots, id, rel) {
        return Err((why, "at first enable".to_string()));
    }
    let path = find_file(roots, id, rel).ok_or_else(|| (GreyReason::Missing(rel.to_string()), "at first enable".to_string()))?;
    std::fs::read(path).map_err(|e| (GreyReason::Bad(rel.to_string()), e.to_string()))
}

/// The mod's model and clips from the first root that has them, or the file that's wrong and why.
fn load_hero_assets(roots: &[PathBuf], id: &str) -> Result<HeroAssets, FileFault> {
    const MODEL: &str = "omniman.sevm";
    const CLIPS: &str = "clips.sevc";
    let refused = |rel: &str, e: hero_asset::HeroAssetError| (GreyReason::Bad(rel.to_string()), e.to_string());
    let model = hero_asset::parse_model(&read_need(roots, id, MODEL)?).map_err(|e| refused(MODEL, e))?;
    let clips = hero_asset::parse_clips(&read_need(roots, id, CLIPS)?).map_err(|e| refused(CLIPS, e))?;
    hero_asset::check_pair(&model, &clips).map_err(|e| refused(CLIPS, e))?;
    Ok(HeroAssets { model, clips })
}

/// Looks inside one file by its kind, as the part of the game that will use it reads it. Rubbish in a texture's
/// blocks passes (any bytes are blocks) and draws as rubbish; everything that could crash or fail to load doesn't.
/// The model and clips are `load_hero_assets`'s; any other kind has only the scan's checks.
fn check_inside(rel: &str, bytes: Vec<u8>) -> Result<(), String> {
    match std::path::Path::new(rel).extension().and_then(|e| e.to_str()) {
        Some("dds") => se_core::dds::parse_dds(&bytes).map(|_| ()).map_err(|e| e.to_string()),
        Some("sehm") => hero_asset::check_sehm(&bytes).map_err(|e| e.to_string()),
        Some("ogg") => check_sound(bytes).map(|_| ()),
        _ => Ok(()),
    }
}

/// First enable: every file the mod needs is still there and is what it should be. The first wrong one greys the
/// row, so a damaged file is found here and not when a later part of the mod first reaches for it.
fn check_files(roots: &[PathBuf], id: &str, needs: &[String]) -> Result<(), FileFault> {
    for rel in needs {
        check_inside(rel, read_need(roots, id, rel)?).map_err(|why| (GreyReason::Bad(rel.clone()), why))?;
    }
    Ok(())
}

/// What a sound decodes to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundFacts {
    pub channels: u32,
    pub rate: u32,
    pub seconds: f32,
    pub peak: f32,
}

/// A mod's sound as a source the game can play, decoded once with the same decoder as playing it. bevy_audio
/// panics on anything but Ogg Vorbis (spec, F10) and on a file whose headers are cut short: the guard goes first,
/// and a panic of the decoder's here is only a refusal, before anything tries to play it.
pub fn check_sound(bytes: Vec<u8>) -> Result<(AudioSource, SoundFacts), String> {
    if !crate::music::is_ogg_vorbis(&bytes) {
        return Err("not an Ogg Vorbis file".to_string());
    }
    let source = AudioSource { bytes: std::sync::Arc::from(bytes) };
    let decode = std::panic::AssertUnwindSafe(|| {
        let decoder = source.decoder();
        let (channels, rate) = (u32::from(decoder.channels().get()), decoder.sample_rate().get());
        let (mut samples, mut peak) = (0u64, 0.0f32);
        for s in decoder {
            samples += 1;
            peak = peak.max(s.abs());
        }
        (channels, rate, samples, peak)
    });
    let (channels, rate, samples, peak) = std::panic::catch_unwind(decode).map_err(|_| "its decoder refused it".to_string())?;
    if samples == 0 {
        return Err("no samples".to_string());
    }
    let seconds = samples as f32 / (channels.max(1) * rate.max(1)) as f32;
    Ok((source, SoundFacts { channels, rate, seconds, peak }))
}

/// A sound the dev command `mods sound` started: the log says when it really plays and when it ends.
#[derive(Resource)]
struct DevSound {
    entity: Entity,
    name: String,
    playing: bool,
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
            // After whatever can start piloting this frame (`ship_input` and the commands chained before it:
            // Enter, F, the Ships window; `move_mode_keys`: key 4), and before a script's first command: the
            // mod goes off first.
            .add_systems(
                Update,
                (off_for_pilot_or_script.run_if(mod_on), apply_request.run_if(has_request))
                    .chain()
                    .after(crate::ship::ship_input)
                    .after(crate::camera::move_mode_keys)
                    .after(crate::camera::input_controls)
                    .before(crate::scripting::run_script),
            )
            // After bevy_window turns the closed window into AppExit, before the session is saved.
            .add_systems(Last, off_on_exit.after(bevy::window::ExitSystems).before(crate::session::save_on_exit).run_if(mod_on))
            .add_systems(Update, watch_dev_sound.run_if(resource_exists::<DevSound>))
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
    let mut info = ModInfo { id: id.to_string(), name: id.to_string(), description: String::new(), code: String::new(), fault: None, needs: Vec::new() };
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
    info.needs = cfg.all("Need").filter_map(|need| need.str()).map(str::to_string).collect();
    info
}

/// What's wrong with a file a mod needs, if anything. A path that leaves the mod's folder is bad, and an
/// empty file is: a copy cut short. First enable looks inside (`check_files`, `load_hero_assets`).
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

fn apply_request(
    mut commands: Commands,
    mut control: ResMut<ModControl>,
    mut reg: ResMut<ModRegistry>,
    flight: Res<Flight>,
    runner: Res<ScriptRunner>,
    hero: Option<Res<HeroAssets>>,
) {
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
            // First enable: look inside the files. A refusal greys the row for good.
            if m.code == "viltrumite" && hero.is_none() {
                let started = std::time::Instant::now();
                let roots = mod_roots();
                let checked = check_files(&roots, &id, &m.needs);
                let check_ms = started.elapsed().as_millis();
                let loaded = checked.and_then(|()| load_hero_assets(&roots, &id));
                match loaded {
                    Ok(assets) => {
                        let (model, clips) = (&assets.model, &assets.clips);
                        info!(
                            "mods: {id}: {} bones, {} draws, {} vertices, {} triangles; {} clips; {} files checked in {check_ms} ms, all read in {} ms",
                            model.bones.len(),
                            model.draws.len(),
                            model.vertices.len(),
                            model.indices.len() / 3,
                            clips.clips.len(),
                            m.needs.len(),
                            started.elapsed().as_millis()
                        );
                        if clips.stand_ins() > 0 {
                            warn!("mods: {id}: {} clips are stand-ins until the Mixamo ones are converted", clips.stand_ins());
                        }
                        commands.insert_resource(assets);
                    }
                    Err((grey, why)) => {
                        warn!("mods: {id}: {grey}: {why}");
                        reg.mark(&id, grey);
                        return;
                    }
                }
            }
            let name = reg.name_of(&id).to_string();
            let is_hero = m.code == "viltrumite";
            control.off(&reg, "another mod turned on");
            info!("mods: {name} on");
            control.active = Some(id);
            control.dev = dev;
            control.hero = is_hero;
            control.loading = is_hero;
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
            ui.label(i18n.tr("No mods found")); // (weak text is nearly invisible under the skin)
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

/// Plays a mod's sound once: `sfx/NAME.ogg`, as a test run's proof that its files play in the game.
fn dev_play_sound(world: &mut World, name: &str) {
    let rel = format!("sfx/{name}.ogg");
    let Some(path) = mod_file("viltrumite", &rel) else {
        warn!("mods: sound {rel} not found");
        return;
    };
    let checked = std::fs::read(&path).map_err(|e| e.to_string()).and_then(check_sound);
    let (source, facts) = match checked {
        Ok(ok) => ok,
        Err(why) => {
            warn!("mods: sound {rel}: {why}");
            return;
        }
    };
    info!("mods: sound {rel}: decoded {} channels, {} Hz, {:.2} s, peak {:.2}", facts.channels, facts.rate, facts.seconds, facts.peak);
    let volume = world.resource::<crate::sound::SoundSettings>().volume;
    let handle = world.resource_mut::<Assets<AudioSource>>().add(source);
    let entity = world.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)))).id();
    world.insert_resource(DevSound { entity, name: rel, playing: false });
}

fn watch_dev_sound(mut commands: Commands, mut sound: ResMut<DevSound>, sinks: Query<(), With<AudioSink>>, alive: Query<()>) {
    if !sound.playing && sinks.contains(sound.entity) {
        sound.playing = true;
        info!("mods: sound {} is playing", sound.name);
    }
    if !alive.contains(sound.entity) {
        info!("mods: sound {} finished", sound.name);
        commands.remove_resource::<DevSound>();
    }
}

/// The dev command `mods list | on NAME | off | window [off] | sound NAME` (camera.rs's match).
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
        "sound" => dev_play_sound(world, name),
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

    /// The project's own mods folder.
    fn tree() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("mods")
    }

    #[test]
    fn the_real_mod_is_found_with_every_need_there() {
        let m = read_mod(&[tree()], "viltrumite");
        assert_eq!(m.fault, None);
        assert_eq!(m.code, "viltrumite");
        let cfg = se_core::sc::parse(&fs::read_to_string(tree().join("viltrumite").join("mod.cfg")).unwrap());
        let needs: Vec<&str> = cfg.all("Need").filter_map(|n| n.str()).collect();
        for must in ["omniman.sevm", "clips.sevc", "sfx/sonic_boom_v2.ogg", "props/nibsuper_rock1.sehm", "sprites/ptfx_smoke_wispy_anim.dds"] {
            assert!(needs.contains(&must), "{must} isn't a Need");
        }
        assert_eq!(needs.iter().filter(|n| n.starts_with("sfx/")).count(), 29, "the plan's sound tables");
    }

    #[test]
    fn the_real_model_and_clips_load() {
        let assets = load_hero_assets(&[tree()], "viltrumite").unwrap();
        assert_eq!(assets.model.bones.len(), 128);
        assert!(assets.clips.clip("bf_idle").is_some());
    }

    /// A file the parsers refuse names itself; the registry greys the row with it (`apply_request`). One that isn't
    /// there says Missing, not Bad, whether it went after the scan or `mod.cfg` never named it.
    #[test]
    fn a_bad_model_or_clip_file_is_named_and_greys_the_row() {
        let root = scratch("bad_hero");
        let real = tree().join("viltrumite");
        let copy = |name: &str| fs::copy(real.join(name), root.join("m").join(name)).unwrap();
        write(&root, "m", "mod.cfg", "Name \"x\"\nCode \"viltrumite\"\n");
        assert_eq!(load_hero_assets(std::slice::from_ref(&root), "m").err().unwrap().0, GreyReason::Missing("omniman.sevm".into()));
        write(&root, "m", "omniman.sevm", "not a model");
        let (grey, why) = load_hero_assets(std::slice::from_ref(&root), "m").err().unwrap();
        assert_eq!(grey, GreyReason::Bad("omniman.sevm".into()));
        assert!(why.contains("SEVM"), "{why}");
        copy("omniman.sevm");
        assert_eq!(load_hero_assets(std::slice::from_ref(&root), "m").err().unwrap().0, GreyReason::Missing("clips.sevc".into()));
        write(&root, "m", "clips.sevc", "SEVC and then nonsense");
        assert_eq!(load_hero_assets(std::slice::from_ref(&root), "m").err().unwrap().0, GreyReason::Bad("clips.sevc".into()));
        copy("clips.sevc");
        assert!(load_hero_assets(std::slice::from_ref(&root), "m").is_ok());

        let mut reg = ModRegistry { mods: vec![read_mod(std::slice::from_ref(&root), "m")] };
        assert_eq!(reg.mods[0].fault, None, "the Need check only looks at sizes");
        reg.mark("m", GreyReason::Bad("clips.sevc".into()));
        assert_eq!(grey_reason(&reg.mods[0], false, false).unwrap().to_string(), "Bad clips.sevc");
        let _ = fs::remove_dir_all(&root);
    }

    /// Rubbish in, or a cut-short copy of, each kind of file the mod has: its own path greys the row, whichever
    /// file is damaged. A real file of each kind passes. The whole real mod passes too.
    #[test]
    fn first_enable_looks_inside_every_kind_of_file() {
        let root = scratch("kinds");
        let real = tree().join("viltrumite");
        let roots = [root.clone()];
        for rel in ["tex/feet_normal_000.dds", "sprites/ptfx_smoke_wispy_anim.dds", "props/nibsuper_rock1.sehm", "sfx/punch_1.ogg"] {
            let whole = fs::read(real.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
            let needs = [rel.to_string()];
            fs::create_dir_all(root.join("m").join(rel).parent().unwrap()).unwrap();
            fs::write(root.join("m").join(rel), &whole).unwrap();
            check_files(&roots, "m", &needs).unwrap_or_else(|e| panic!("{rel}: {e:?}"));
            for damaged in [b"a few bytes of rubbish".to_vec(), vec![0xA5; 4096]] {
                fs::write(root.join("m").join(rel), &damaged).unwrap();
                let (grey, why) = check_files(&roots, "m", &needs).err().unwrap_or_else(|| panic!("{rel} passed with rubbish in it"));
                assert_eq!(grey, GreyReason::Bad(rel.to_string()), "{why}");
            }
            fs::remove_file(root.join("m").join(rel)).unwrap();
            assert_eq!(check_files(&roots, "m", &needs).err().unwrap().0, GreyReason::Missing(rel.to_string()), "gone since the scan");
        }
        // Cut short: the textures' and props' sizes no longer add up. (A sound cut short may still play, shorter.)
        for rel in ["tex/feet_normal_000.dds", "props/nibsuper_rock1.sehm"] {
            let whole = fs::read(real.join(rel)).unwrap();
            fs::create_dir_all(root.join("m").join(rel).parent().unwrap()).unwrap();
            fs::write(root.join("m").join(rel), &whole[..whole.len() - 1]).unwrap();
            assert_eq!(check_files(&roots, "m", &[rel.to_string()]).err().unwrap().0, GreyReason::Bad(rel.to_string()));
        }
        let _ = fs::remove_dir_all(&root);

        let m = read_mod(&[tree()], "viltrumite");
        assert_eq!(m.needs.len(), 55);
        let started = std::time::Instant::now();
        check_files(&[tree()], "viltrumite", &m.needs).unwrap();
        println!("the whole mod checked in {} ms", started.elapsed().as_millis());
    }

    /// Every sound through the guard and the real decoder, as `mods sound` and step 6 play them. The loops: at
    /// most about 10 s (the plan's trim), no silence at the end, and no click at the seam.
    #[test]
    fn every_sound_decodes_and_the_loops_close() {
        let dir = tree().join("viltrumite").join("sfx");
        let mut count = 0;
        for e in fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let bytes = fs::read(e.path()).unwrap();
            let (source, facts) = check_sound(bytes).unwrap_or_else(|why| panic!("{name}: {why}"));
            assert!(facts.seconds > 0.1 && facts.peak > 0.01, "{name}: {facts:?}");
            count += 1;
            if name.contains("_loop") {
                assert!(facts.seconds <= 10.2, "{name}: {facts:?}");
                let samples: Vec<f32> = source.decoder().collect();
                let ch = facts.channels as usize;
                let tail = &samples[samples.len() - 2048 * ch..];
                let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
                let whole = rms(&samples);
                assert!(rms(tail) > 0.2 * whole, "{name}: the end is quiet ({} against {whole}): padding would gap the loop", rms(tail));
                let jump = (0..ch).map(|c| (samples[samples.len() - ch + c] - samples[c]).abs()).fold(0.0f32, f32::max);
                println!("{name}: {facts:?}, tail {} of {whole}, seam jump {jump}", rms(tail));
                assert!(jump < 6.0 * rms(tail).max(0.01) * 0.5 + 0.08, "{name}: a click at the seam, {jump}");
            }
        }
        assert_eq!(count, 29);
    }

    #[test]
    fn what_isnt_ogg_vorbis_is_refused_before_the_decoder() {
        assert!(check_sound(b"RIFF....WAVEfmt ".to_vec()).is_err());
        assert!(check_sound(Vec::new()).is_err());
        assert!(check_sound(b"OggS and not vorbis".to_vec()).is_err());
    }

    /// A sound cut short at any point: refused or a shorter sound, never a panic out of here.
    #[test]
    fn a_sound_cut_short_never_panics() {
        let whole = fs::read(tree().join("viltrumite").join("sfx").join("punch_1.ogg")).unwrap();
        let (_, full) = check_sound(whole.clone()).unwrap();
        for n in [10, 40, 60, 100, 300, 1000, 3000, whole.len() / 2, whole.len() - 1] {
            if let Ok((_, cut)) = check_sound(whole[..n].to_vec()) {
                assert!(cut.seconds <= full.seconds, "{n} bytes: {cut:?}");
            }
        }
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
        let mut m = ModInfo { id: "m".into(), name: "M".into(), description: String::new(), code: "viltrumite".into(), fault: None, needs: Vec::new() };
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
            mods: vec![ModInfo { id: "m".into(), name: "M".into(), description: String::new(), code: "viltrumite".into(), fault: None, needs: Vec::new() }],
        };
        reg.mark("m", GreyReason::Bad("omniman.sevm".into()));
        reg.mark("nobody", GreyReason::Bad("x".into()));
        assert_eq!(grey_reason(reg.get("m").unwrap(), false, false), Some(GreyReason::Bad("omniman.sevm".into())));
        reg.mark("m", GreyReason::Missing("sfx/a.ogg".into()));
        assert_eq!(grey_reason(reg.get("m").unwrap(), false, false).unwrap().to_string(), "Missing sfx/a.ogg");
    }

    /// The two systems that decide what is on, without the rest of the game.
    /// Empty assets: these tests turn mods on and off without reading the real files.
    fn no_assets() -> HeroAssets {
        let model = HeroModel { bones: vec![], draws: vec![], vertices: vec![], indices: vec![], cape_first: None, cape_chains: 0, cape_links: 0, bounds_min: Vec3::ZERO, bounds_max: Vec3::ZERO };
        let clips = ClipSet { fps: 30.0, bone_count: 0, clips: vec![], tracks: vec![], events: vec![], quats: vec![], roots: vec![] };
        HeroAssets { model, clips }
    }

    /// First enable: the real files parse and stay; a folder whose files don't turns grey and stays off.
    #[test]
    fn first_enable_reads_the_files_and_a_refusal_greys_the_row() {
        let mut app = control_app(&[("viltrumite", None), ("no_files_here", None)]);
        app.world_mut().remove_resource::<HeroAssets>();
        ask(&mut app, on("no_files_here"));
        assert_eq!(active(&app), None);
        let row = app.world().resource::<ModRegistry>().get("no_files_here").unwrap().clone();
        assert_eq!(grey_reason(&row, false, false).unwrap().to_string(), "Missing omniman.sevm");
        assert!(!app.world().contains_resource::<HeroAssets>());
        ask(&mut app, on("viltrumite"));
        assert_eq!(active(&app).as_deref(), Some("viltrumite"));
        app.update();
        let assets = app.world().resource::<HeroAssets>();
        assert_eq!(assets.model.bones.len(), 128);
        assert!(!assets.clips.clips.is_empty());
    }

    fn control_app(mods: &[(&str, Option<GreyReason>)]) -> App {
        let mut app = App::new();
        let mods = mods
            .iter()
            .map(|(id, fault)| ModInfo { id: id.to_string(), name: id.to_string(), description: String::new(), code: "viltrumite".into(), fault: fault.clone(), needs: Vec::new() })
            .collect();
        app.insert_resource(ModRegistry { mods })
            .insert_resource(no_assets())
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
        // Viltrumite's code: the hero's systems run, and the row reads Loading until he can show.
        let control = app.world().resource::<ModControl>();
        assert!(control.hero && control.loading);
        // One at a time; a grey or unknown mod changes nothing; off is always allowed.
        ask(&mut app, on("b"));
        assert_eq!(active(&app).as_deref(), Some("b"));
        ask(&mut app, on("grey"));
        ask(&mut app, on("nobody"));
        assert_eq!(active(&app).as_deref(), Some("b"));
        ask(&mut app, ModRequest::Off);
        assert_eq!(active(&app), None);
        let control = app.world().resource::<ModControl>();
        assert!(!control.hero && !control.loading);
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

    /// The plugin's own order against the real key 4, whichever way round the systems were added: the mod
    /// is off in the very frame piloting starts (flow F3), and asking for it that frame changes nothing.
    #[test]
    fn key_4_turns_the_mod_off_in_the_frame_piloting_starts() {
        for camera_first in [true, false] {
            for already_on in [true, false] {
                let mut app = App::new();
                app.init_resource::<ButtonInput<KeyCode>>()
                    .init_resource::<crate::camera::FlyCamera>()
                    .init_resource::<crate::messages::Messages>()
                    .init_resource::<bevy_egui::EguiUserTextures>()
                    .init_resource::<Flight>()
                    .init_resource::<ScriptRunner>()
                    .insert_resource(crate::i18n::I18n::new(&se_core::vfs::Vfs::new(), "eng"));
                if camera_first {
                    app.add_systems(Update, crate::camera::move_mode_keys).add_plugins(ModsPlugin);
                } else {
                    app.add_plugins(ModsPlugin).add_systems(Update, crate::camera::move_mode_keys);
                }
                app.insert_resource(no_assets());
                app.update(); // Startup's scan of the real folders: the test's own list replaces it.
                app.world_mut().resource_mut::<ModRegistry>().mods =
                    vec![ModInfo { id: "a".into(), name: "a".into(), description: String::new(), code: "viltrumite".into(), fault: None, needs: Vec::new() }];
                if already_on {
                    ask(&mut app, on("a"));
                    assert_eq!(active(&app).as_deref(), Some("a"));
                } else {
                    app.world_mut().resource_mut::<ModControl>().request = Some(on("a"));
                }
                let ship = app.world_mut().spawn_empty().id();
                app.world_mut().resource_mut::<Flight>().ship = Some(ship);
                app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Digit4);
                app.update();
                assert!(app.world().resource::<Flight>().active, "key 4 pilots the parked ship");
                assert_eq!(active(&app), None, "camera_first {camera_first}, already_on {already_on}");
            }
        }
    }
}
