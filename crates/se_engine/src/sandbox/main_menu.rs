//! The main menu (the user's 2026-11-12), laid out as Garry's Mod's: a full-screen picture that slowly changes, the title at the top
//! left, a column of plain text entries down the left that light up under the mouse, a thin bar along the bottom. Its own title,
//! nothing of Garry's Mod's but the layout (read from its `html/` folder, nothing copied) and the Q menu's colours and sounds.
//!
//! - **When:** after the loading screen of every launch the user makes (it is up from the start, drawn under the loading screen,
//!   and takes no click until that has gone). Test runs (`--run`, `--screenshot`, `--script`) skip it, or every script would hang:
//!   `SE_MENU=1` shows it in one, `SE_MENU=0` skips it in a normal launch. Without the sandbox (`SE_SANDBOX=0`) there is none.
//! - **Entries:** Resume (only once playing), Play (the first time: into the game, already loading behind the menu; once playing:
//!   back on Earth's ground on foot, as a launch starts), Mods (who to play as: the Q menu's own rows), Options (the settings
//!   window), Quit.
//! - **Esc** opens it while playing, and under it closes Options, then the Mods page, then the menu. While a ship is flown Esc is
//!   still the ship's (it leaves flight), and a text field keeps it.
//! - **Input:** while it's up `menu_input` takes every key and mouse button before anything reads them (only Esc's held state is
//!   kept, or the key's auto-repeat would press it again), and `ModControl::main_menu` frees the cursor and gives egui the mouse
//!   in walking (`sandbox::read_input`) and for a mod (`mods::snapshot_input`). The world goes on behind it: a mod's hero flying
//!   keeps flying, the walker stands.
//! - **Pictures:** the user's own screenshots (`Pictures\SpaceEngine-rs` and its `screenshots-hd`, never `export`: sky faces),
//!   else Garry's Mod's backgrounds from the pack of its files (`assets/gmod.pak`, `backgrounds/`), else plain dark. Listed and decoded on
//!   worker threads, two at a time in memory, each shown `HOLD_S` with a slow zoom and crossfaded over `FADE_S`.

use std::path::{Path, PathBuf};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiContexts, egui};

use crate::config::EngineConfig;
use crate::data::SeData;
use crate::mods::{ModControl, ModRegistry};
use crate::universe::SimTime;

use super::assets::GmodAssets;
use super::audio::Cue;
use super::menu::look;
use super::{Mode, Sandbox};

/// Each picture's time on screen (s) and its crossfade into the next.
const HOLD_S: f32 = 12.0;
const FADE_S: f32 = 3.0;
/// How much a picture grows over its time (the slow zoom), and how far it drifts (a share of what the zoom leaves over).
const ZOOM: f32 = 0.08;
/// Pictures are kept at most this size (texels), whatever the file.
const MAX_W: u32 = 1920;
const MAX_H: u32 = 1200;
/// Wider than tall by at least this: a sky cube's face or a portrait isn't a background.
const MIN_ASPECT: f32 = 1.2;
/// The bottom bar's height and the column's left edge (points).
const BAR_H: f32 = 44.0;
const LEFT: f32 = 56.0;
/// Behind everything until a picture is there.
const DARK: egui::Color32 = egui::Color32::from_rgb(16, 18, 22);

/// `SE_MENU`: `1` shows it in a test run too, `0` never.
fn env_switch() -> Option<bool> {
    std::env::var("SE_MENU").ok().map(|v| v.trim() != "0")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Page {
    #[default]
    Main,
    Mods,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Entry {
    Resume,
    Play,
    Mods,
    Options,
    Quit,
    /// The bar's way back from a page (logged with the entries, for a test's `uiclick`).
    Back,
}

impl Entry {
    fn label(self) -> &'static str {
        match self {
            Entry::Resume => "Resume",
            Entry::Play => "Play",
            Entry::Mods => "Mods",
            Entry::Options => "Options",
            Entry::Quit => "Quit",
            Entry::Back => "Back",
        }
    }
}

/// Where the list of pictures comes from, step by step.
#[derive(Default)]
enum Listing {
    #[default]
    NotStarted,
    Own(Task<Vec<Pic>>),
    /// None of the user's: Garry's Mod's, once the search for its install has ended.
    WaitGmod,
    Gmod(Task<Vec<Pic>>),
    Done,
}

/// One picture to show: a file of the user's, or one in the pack of Garry's Mod's files.
#[derive(Clone)]
enum Pic {
    File(PathBuf),
    Gmod(std::sync::Arc<se_core::gmod::GmodInstall>, String),
}

impl Pic {
    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Pic::File(p) => std::fs::read(p).ok(),
            Pic::Gmod(install, key) => install.read(key),
        }
    }
}

#[derive(Default)]
struct Pictures {
    listing: Listing,
    paths: Vec<Pic>,
    from: &'static str,
    /// The next path to decode.
    next_path: usize,
    /// The picture being decoded, with its seed (its drift's direction, kept with it from decoding to the end of its time).
    job: Option<(usize, Task<Option<egui::ColorImage>>)>,
    current: Option<(egui::TextureHandle, usize)>,
    next: Option<(egui::TextureHandle, usize)>,
    /// The current one's time on screen (s), and the first one's fade in from dark.
    age: f32,
    first_in: f32,
    /// Pictures that failed or didn't fit, in a row: the list is given up after a whole round of them.
    misses: usize,
}

#[derive(Resource, Default)]
pub struct MainMenu {
    /// This launch has the menu: Esc opens it.
    pub enabled: bool,
    pub up: bool,
    /// Play has been pressed: Resume shows, and Play puts him back on Earth.
    pub in_game: bool,
    page: Page,
    /// Play while playing: back on Earth's ground on foot, done by `restart_on_earth` over the next frames.
    restart: bool,
    /// The Options window was opened from here: it closes with the menu.
    opened_settings: bool,
    /// The user's own hide-the-interface switch while the menu has it on.
    saved_hidden: Option<bool>,
    pictures: Pictures,
    hovered: Option<Entry>,
    /// The entries as drawn last frame (points), for a test's `uiclick`.
    entries: Vec<(Entry, egui::Rect)>,
    mods_panel: Option<egui::Rect>,
}

impl MainMenu {
    fn open(&mut self, why: &str) {
        if !self.up {
            self.up = true;
            self.page = Page::Main;
            info!("main menu: open ({why})");
        }
    }

    fn close(&mut self, why: &str, hud: &mut crate::hud::HudState) {
        if self.up {
            self.up = false;
            self.in_game = true;
            self.page = Page::Main;
            if std::mem::take(&mut self.opened_settings) {
                hud.show_settings = false;
            }
            info!("main menu: closed ({why})");
        }
    }
}

pub(super) fn build(app: &mut App) {
    app.add_systems(Startup, prepare)
        .add_systems(
            PreUpdate,
            // Before anything reads the keys: the sandbox's (V first of all), the mod's snapshot, the camera's and the panels'.
            menu_input
                .after(bevy::input::InputSystems)
                .after(crate::viltrumite::dev_keys)
                .before(super::read_input)
                .before(crate::mods::snapshot_input),
        )
        .add_systems(Update, (restart_on_earth, log_cursor))
        .add_systems(bevy_egui::EguiPrimaryContextPass, draw.after(super::menu::draw_menu).in_set(crate::skin::UiSet::Draw));
}

fn prepare(mut menu: ResMut<MainMenu>, cfg: Res<EngineConfig>) {
    menu.enabled = env_switch().unwrap_or_else(|| !crate::settings::scripted(&cfg));
    menu.up = menu.enabled;
    if menu.enabled {
        info!("main menu: on for this launch (SE_MENU=0 skips it)");
    }
}

/// Esc, and every key and button taken while the menu is up.
#[allow(clippy::too_many_arguments)]
fn menu_input(
    mut menu: ResMut<MainMenu>,
    mut control: ResMut<ModControl>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    (flight, splash, mut hud): (Res<crate::ship::Flight>, Res<crate::splash::Splash>, ResMut<crate::hud::HudState>),
    mut egui: EguiContexts,
) {
    if !menu.enabled {
        if control.main_menu {
            control.main_menu = false;
        }
        return;
    }
    let typing = egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false);
    // (While a ship is flown Esc leaves flight, ship.rs, and a text field keeps it. With the menu up it is always the menu's: a
    // focused entry or a ship somehow flown must not leave it without a way out.)
    let ours = menu.up || (!typing && !flight.active);
    if keys.just_pressed(KeyCode::Escape) && ours && !splash.covering() {
        keys.clear_just_pressed(KeyCode::Escape);
        if !menu.up {
            menu.open("Esc");
        } else if hud.show_settings {
            hud.show_settings = false;
            menu.opened_settings = false;
            info!("main menu: Options closed (Esc)");
        } else if menu.page != Page::Main {
            menu.page = Page::Main;
        } else if menu.in_game {
            menu.close("Esc", &mut hud);
        }
    }
    if menu.up {
        // Esc's held state stays: taken away, the key's auto-repeat would press it anew and close the menu again.
        let esc = keys.pressed(KeyCode::Escape);
        keys.reset_all();
        if esc {
            keys.press(KeyCode::Escape);
            keys.clear_just_pressed(KeyCode::Escape);
        }
        buttons.reset_all();
        motion.delta = Vec2::ZERO;
        scroll.delta = Vec2::ZERO;
        // The port's own windows and toolbars (Find, Places, Help, the star browser, Ships, the scripts' toolbars, the ship's
        // HUD) go with its hide-the-interface switch, which the Options window doesn't: on while the menu is up, the user's
        // own setting back after.
        if menu.saved_hidden.is_none() {
            menu.saved_hidden = Some(hud.hidden);
        }
        if !hud.hidden {
            hud.hidden = true;
        }
    } else if let Some(was) = menu.saved_hidden.take() {
        hud.hidden = was;
    }
    if control.main_menu != menu.up {
        control.main_menu = menu.up;
    }
}

/// Play while playing: the mod goes first (its hero hands back to the free flight), then he's put on Earth's ground on foot.
#[allow(clippy::too_many_arguments)]
fn restart_on_earth(
    mut menu: ResMut<MainMenu>,
    mut control: ResMut<ModControl>,
    mut sb: ResMut<Sandbox>,
    mut cam: ResMut<crate::camera::FlyCamera>,
    mut flight: ResMut<crate::ship::Flight>,
    data: Res<SeData>,
    time: Res<SimTime>,
) {
    if !menu.restart {
        return;
    }
    if flight.active {
        // (As Esc leaves flight: the ship stays where it is.)
        flight.active = false;
        flight.stop_engines();
    }
    if control.active.is_some() {
        control.request_off();
        sb.before_hero = Mode::Fly;
        return;
    }
    if sb.mode == Mode::Hero {
        // (The sandbox drops to free flight the frame after the mod goes.)
        return;
    }
    menu.restart = false;
    if !super::put_on_earth(&data, time.jd, &mut cam, &mut sb, "the main menu's Play: back") {
        warn!("main menu: Play: no Earth in the catalog");
    }
}

/// A real mouse's test can't see the cursor: the log says what the grab became.
fn log_cursor(menu: Res<MainMenu>, cursor: Single<&CursorOptions, With<PrimaryWindow>>, mut last: Local<Option<(bevy::window::CursorGrabMode, bool, bool)>>) {
    if !menu.enabled {
        return;
    }
    let now = (cursor.grab_mode, cursor.visible, menu.up);
    if *last != Some(now) {
        *last = Some(now);
        info!("main menu: cursor {:?}, visible {}, menu up {}", now.0, now.1, now.2);
    }
}

/// The user's screenshots: the folder and its `screenshots-hd`, never `export` (sky faces, cylinder maps).
fn own_pictures(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in [folder.to_path_buf(), folder.join("screenshots-hd")] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && is_picture(&p) {
                out.push(p);
            }
        }
    }
    out
}

fn is_picture(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png"))
}

/// Garry's Mod's backgrounds, in the pack of its files.
fn gmod_pictures(install: std::sync::Arc<se_core::gmod::GmodInstall>) -> Vec<Pic> {
    install.folder_files("backgrounds").into_iter().filter(|k| is_picture(Path::new(k))).map(|k| Pic::Gmod(install.clone(), k)).collect()
}

/// A picture as a texture's pixels, at most `MAX_W` x `MAX_H`; `None` if it can't be read or isn't wide.
fn decode(pic: &Pic) -> Option<egui::ColorImage> {
    let bytes = pic.bytes()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || (w as f32) < MIN_ASPECT * h as f32 {
        return None;
    }
    let img = if w > MAX_W || h > MAX_H { img.resize(MAX_W, MAX_H, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// A different order each launch.
fn shuffle<T>(paths: &mut [T]) {
    let mut s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9E37_79B9, |d| d.as_nanos() as u64) | 1;
    for i in (1..paths.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        paths.swap(i, (s % (i as u64 + 1)) as usize);
    }
}

/// The list, the decoding and the crossfade, a step a frame.
fn pump_pictures(p: &mut Pictures, ctx: &egui::Context, assets: &GmodAssets, dt: f32) {
    let pool = AsyncComputeTaskPool::get();
    match &mut p.listing {
        Listing::NotStarted => {
            let folder = crate::screenshot::folder();
            p.listing = Listing::Own(pool.spawn(async move { own_pictures(&folder).into_iter().map(Pic::File).collect() }));
        }
        Listing::Own(task) => {
            if let Some(found) = check_ready(task) {
                if found.is_empty() {
                    p.listing = Listing::WaitGmod;
                } else {
                    p.paths = found;
                    p.from = "your screenshots";
                    p.listing = Listing::Done;
                    shuffle(&mut p.paths);
                    info!("main menu: {} pictures from {}", p.paths.len(), p.from);
                }
            }
        }
        Listing::WaitGmod => {
            if assets.looked {
                match assets.install.clone() {
                    Some(install) => p.listing = Listing::Gmod(pool.spawn(async move { gmod_pictures(install) })),
                    None => p.listing = Listing::Done,
                }
            }
        }
        Listing::Gmod(task) => {
            if let Some(found) = check_ready(task) {
                p.paths = found;
                p.from = "Garry's Mod's backgrounds";
                p.listing = Listing::Done;
                shuffle(&mut p.paths);
                info!("main menu: {} pictures from {}", p.paths.len(), p.from);
            }
        }
        Listing::Done => {}
    }
    // One decoded at a time, while a place for it is free.
    if let Some((seed, job)) = p.job.as_mut() {
        let seed = *seed;
        if let Some(done) = check_ready(job) {
            p.job = None;
            match done {
                Some(image) => {
                    p.misses = 0;
                    let tex = ctx.load_texture(format!("main menu picture {seed}"), image, egui::TextureOptions::LINEAR);
                    if p.current.is_none() {
                        p.current = Some((tex, seed));
                        p.age = 0.0;
                    } else {
                        p.next = Some((tex, seed));
                        // Late (a slow file): it still crossfades in, from now.
                        p.age = p.age.min(HOLD_S);
                    }
                }
                None => p.misses += 1,
            }
        }
    }
    // None of the user's could be shown: Garry's Mod's, as if there had been none.
    if p.current.is_none() && p.job.is_none() && p.from == "your screenshots" && p.misses >= p.paths.len() {
        info!("main menu: none of your {} screenshots could be shown", p.paths.len());
        p.paths.clear();
        p.misses = 0;
        p.from = "";
        p.listing = Listing::WaitGmod;
    }
    // Time on screen, and the swap once the crossfade is over (before the next is asked for, so it starts this frame).
    if p.current.is_some() {
        p.age += dt;
        p.first_in = (p.first_in + dt / FADE_S).min(1.0);
        if p.age >= HOLD_S + FADE_S && p.next.is_some() {
            p.current = p.next.take();
            p.age -= HOLD_S + FADE_S;
        }
    }
    let room = p.current.is_none() || (p.next.is_none() && p.paths.len() > 1);
    if p.job.is_none() && room && !p.paths.is_empty() && p.misses < p.paths.len() {
        let path = p.paths[p.next_path % p.paths.len()].clone();
        let seed = p.next_path;
        p.next_path += 1;
        p.job = Some((seed, pool.spawn(async move { decode(&path) })));
    }
}

/// A picture over the whole screen (cover fit), zoomed and drifted by `t` (0..1 of its time), at `alpha`.
fn paint_picture(painter: &egui::Painter, screen: egui::Rect, tex: &egui::TextureHandle, t: f32, seed: usize, alpha: f32) {
    let [w, h] = tex.size();
    let (a, s) = (w as f32 / h.max(1) as f32, screen.width() / screen.height().max(1.0));
    let (mut uw, mut uh) = if a > s { (s / a, 1.0) } else { (1.0, a / s) };
    let zoom = 1.0 + ZOOM * t;
    uw /= zoom;
    uh /= zoom;
    // Each picture drifts its own way.
    let angle = seed as f32 * 2.399;
    let (dx, dy) = (angle.cos() * (t - 0.5), angle.sin() * (t - 0.5));
    let c = egui::pos2(0.5 + dx * (1.0 - uw), 0.5 + dy * (1.0 - uh));
    let uv = egui::Rect::from_center_size(c, egui::vec2(uw, uh));
    painter.image(tex.id(), screen, uv, egui::Color32::WHITE.gamma_multiply(alpha));
}

/// A run of text in the menu's way: a soft shadow under it, and a glow when lit.
fn shadowed(painter: &egui::Painter, pos: egui::Pos2, align: egui::Align2, text: &str, font: egui::FontId, color: egui::Color32, glow: bool) -> egui::Rect {
    let shadow = egui::Color32::from_black_alpha(110);
    painter.text(pos + egui::vec2(1.5, 2.0), align, text, font.clone(), shadow);
    if glow {
        let halo = egui::Color32::from_white_alpha(28);
        for d in [egui::vec2(-1.5, 0.0), egui::vec2(1.5, 0.0), egui::vec2(0.0, -1.5), egui::vec2(0.0, 1.5)] {
            painter.text(pos + d, align, text, font.clone(), halo);
        }
    }
    painter.text(pos, align, text, font, color)
}

#[allow(clippy::too_many_arguments)]
fn draw(
    mut contexts: EguiContexts,
    mut menu: ResMut<MainMenu>,
    mut sb: ResMut<Sandbox>,
    mut control: ResMut<ModControl>,
    mut hud: ResMut<crate::hud::HudState>,
    (assets, reg, flight, runner, splash): (Res<GmodAssets>, Res<ModRegistry>, Res<crate::ship::Flight>, Res<crate::scripting::ScriptRunner>, Res<crate::splash::Splash>),
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let menu = &mut *menu;
    if !menu.enabled {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    if !menu.up {
        menu.hovered = None;
        menu.entries.clear();
        return Ok(());
    }
    let dt = time.delta_secs().min(0.1);
    pump_pictures(&mut menu.pictures, ctx, &assets, dt);
    // Under the loading screen it's drawn but takes nothing.
    let live = !splash.covering();
    let screen = ctx.content_rect();
    let scale = (screen.height() / 900.0).clamp(0.75, 2.0);
    let mut clicked: Option<Entry> = None;
    let mut hovered: Option<Entry> = None;
    let mut entries = Vec::new();
    let mut picked_mod = false;
    let mut back = false;
    let mut mods_panel = None;
    egui::Area::new(egui::Id::new("main_menu")).order(egui::Order::Background).fixed_pos(screen.min).interactable(live).show(ctx, |ui| {
        ui.set_min_size(screen.size());
        // A click anywhere on the picture stops here (the sky's pick behind it must not see it).
        let _ = ui.interact(screen, egui::Id::new("main_menu_backdrop"), egui::Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(screen, 0.0, DARK);
        let p = &menu.pictures;
        if let Some((cur, seed)) = &p.current {
            let span = HOLD_S + FADE_S;
            // (One picture alone zooms in and back out, for ever.)
            let t = if p.paths.len() > 1 { (p.age / span).min(1.0) } else { 1.0 - ((p.age / span) % 2.0 - 1.0).abs() };
            paint_picture(&painter, screen, cur, t, *seed, p.first_in);
            if let (Some((next, next_seed)), true) = (&p.next, p.age > HOLD_S) {
                let k = ((p.age - HOLD_S) / FADE_S).clamp(0.0, 1.0);
                paint_picture(&painter, screen, next, 0.0, *next_seed, k);
            }
        }
        // The column's side darker, so the text reads over any picture.
        let mut shade = egui::Mesh::default();
        let (l, r) = (screen.left(), screen.left() + screen.width() * 0.5);
        let (t, b) = (screen.top(), screen.bottom());
        let dark = egui::Color32::from_black_alpha(150);
        let clear = egui::Color32::TRANSPARENT;
        shade.colored_vertex(egui::pos2(l, t), dark);
        shade.colored_vertex(egui::pos2(r, t), clear);
        shade.colored_vertex(egui::pos2(r, b), clear);
        shade.colored_vertex(egui::pos2(l, b), dark);
        shade.add_triangle(0, 1, 2);
        shade.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(shade));

        // The title.
        let left = screen.left() + LEFT * scale;
        let title_font = egui::FontId::proportional(58.0 * scale);
        let title = egui::pos2(left, screen.top() + 48.0 * scale);
        for d in [0.0, 0.8] {
            shadowed(&painter, title + egui::vec2(d * scale, 0.0), egui::Align2::LEFT_TOP, "Sandbox Engine", title_font.clone(), egui::Color32::WHITE, false);
        }
        // The entries, in groups as Garry's Mod spaces its own.
        let font = egui::FontId::proportional(27.0 * scale);
        let line = 40.0 * scale;
        let gap = 18.0 * scale;
        let mut y = title.y + 58.0 * scale + 46.0 * scale;
        let mut groups: Vec<&[Entry]> = Vec::new();
        if menu.in_game {
            groups.push(&[Entry::Resume]);
        }
        groups.extend([&[Entry::Play, Entry::Mods][..], &[Entry::Options][..], &[Entry::Quit][..]]);
        for group in groups {
            for &e in group {
                let galley_w = painter.layout_no_wrap(e.label().to_string(), font.clone(), look::TEXT).size().x;
                let rect = egui::Rect::from_min_size(egui::pos2(left - 8.0 * scale, y), egui::vec2(galley_w + 16.0 * scale, line));
                let resp = ui.interact(rect, egui::Id::new(("main_menu_entry", e.label())), egui::Sense::click());
                let chosen = (e == Entry::Mods && menu.page == Page::Mods) || (e == Entry::Options && hud.show_settings);
                let lit = live && (resp.hovered() || chosen);
                if live && resp.hovered() {
                    hovered = Some(e);
                }
                let color = if lit { egui::Color32::WHITE } else { egui::Color32::from_white_alpha(205) };
                shadowed(&painter, egui::pos2(left + if lit { 3.0 * scale } else { 0.0 }, y + line * 0.5), egui::Align2::LEFT_CENTER, e.label(), font.clone(), color, lit);
                if live && resp.clicked() {
                    clicked = Some(e);
                }
                entries.push((e, rect));
                y += line;
            }
            y += gap;
        }

        // The Mods page: the Q menu's own rows, in its sheet, beside the column.
        if menu.page == Page::Mods {
            let x0 = left + 260.0 * scale;
            let panel = egui::Rect::from_min_max(egui::pos2(x0, title.y + 58.0 * scale + 40.0 * scale), egui::pos2((x0 + 800.0).min(screen.right() - 40.0), screen.bottom() - BAR_H - 30.0 * scale));
            mods_panel = Some(panel);
            painter.rect_filled(panel, 6.0, look::FRAME.gamma_multiply(0.96));
            painter.rect_stroke(panel, 6.0, egui::Stroke::new(1.5, look::FRAME_EDGE), egui::StrokeKind::Inside);
            let sheet = panel.shrink(8.0);
            painter.rect_filled(sheet, 3.0, look::SHEET);
            ui.scope_builder(egui::UiBuilder::new().max_rect(sheet.shrink(10.0)), |ui| {
                ui.add_enabled_ui(live, |ui| {
                    egui::ScrollArea::vertical().id_salt("main_menu_mods").auto_shrink([false, false]).show(ui, |ui| {
                        picked_mod = super::menu::mod_rows(ui, &mut sb, &mut control, &reg, flight.active, runner.starting(), false, "the main menu's Mods");
                    });
                });
            });
        }

        // The bar along the bottom.
        let bar = egui::Rect::from_min_max(egui::pos2(screen.left(), screen.bottom() - BAR_H * scale), screen.max);
        painter.rect_filled(bar, 0.0, egui::Color32::from_black_alpha(128));
        let small = egui::FontId::proportional(14.0 * scale);
        // On a page, the way back to the main one (as Garry's Mod's bar has it).
        if menu.page != Page::Main {
            let at = bar.left_center() + egui::vec2(LEFT * scale * 0.5, 0.0);
            let w = painter.layout_no_wrap("<  Back to the main menu".to_string(), egui::FontId::proportional(16.0 * scale), look::TEXT).size().x;
            let r = egui::Rect::from_min_size(egui::pos2(at.x - 6.0, bar.top()), egui::vec2(w + 12.0, bar.height()));
            let resp = ui.interact(r, egui::Id::new("main_menu_back"), egui::Sense::click());
            let lit = live && resp.hovered();
            painter.text(at, egui::Align2::LEFT_CENTER, "<  Back to the main menu", egui::FontId::proportional(16.0 * scale), if lit { egui::Color32::WHITE } else { look::DIM });
            if live && resp.clicked() {
                back = true;
            }
            entries.push((Entry::Back, r));
        }
        let hint = match (menu.page, menu.in_game) {
            (Page::Mods, _) => "Esc: back",
            (Page::Main, true) => "Esc: back to the game",
            (Page::Main, false) => "",
        };
        painter.text(bar.right_center() - egui::vec2(LEFT * scale * 0.5, 0.0), egui::Align2::RIGHT_CENTER, hint, small, look::DIM);
    });
    if hovered.is_some() && hovered != menu.hovered {
        sb.cues.push(Cue::new("garrysmod/ui_hover.wav", 0.25, 1.0));
    }
    menu.hovered = hovered;
    menu.entries = entries;
    menu.mods_panel = mods_panel;
    if picked_mod {
        menu.close("Mods: picked who to play as", &mut hud);
    }
    if back {
        sb.cues.push(Cue::ui("garrysmod/ui_click.wav"));
        menu.page = Page::Main;
        info!("main menu: back to the main page");
    }
    if let Some(e) = clicked {
        sb.cues.push(Cue::ui("garrysmod/ui_click.wav"));
        info!("main menu: {}", e.label());
        match e {
            Entry::Resume => menu.close("Resume", &mut hud),
            Entry::Play => {
                if menu.in_game {
                    menu.restart = true;
                }
                menu.close("Play", &mut hud);
            }
            Entry::Mods => {
                menu.page = if menu.page == Page::Mods { Page::Main } else { Page::Mods };
                // (One at a time: the Options window would cover the page.)
                if menu.page == Page::Mods && hud.show_settings {
                    hud.show_settings = false;
                    menu.opened_settings = false;
                }
            }
            Entry::Options => {
                hud.show_settings = !hud.show_settings;
                menu.opened_settings = hud.show_settings;
                if hud.show_settings {
                    menu.page = Page::Main;
                }
            }
            Entry::Quit => {
                exit.write(AppExit::Success);
            }
            Entry::Back => {}
        }
    }
    ctx.request_repaint();
    Ok(())
}

/// `menu on|off|state|mods|main`: open it, close it (as Play does), log it (the entries' places for `uiclick`), its pages.
pub fn dev_command(world: &mut World, arg: &str) {
    let verb = arg.split_whitespace().next().unwrap_or("state").to_ascii_lowercase();
    if !world.contains_resource::<MainMenu>() {
        return;
    }
    world.resource_scope(|world, mut menu: Mut<MainMenu>| match verb.as_str() {
        "on" => {
            menu.enabled = true;
            menu.open("a dev command");
        }
        "off" => {
            let mut hud = world.resource_mut::<crate::hud::HudState>();
            menu.close("a dev command", &mut hud);
        }
        "mods" => menu.page = Page::Mods,
        "main" => menu.page = Page::Main,
        _ => {
            let entries: Vec<String> = menu.entries.iter().map(|(e, r)| format!("{} at {:.0},{:.0} ({:.0}x{:.0})", e.label(), r.center().x, r.center().y, r.width(), r.height())).collect();
            let p = &menu.pictures;
            info!(
                "main menu: enabled {}, up {}, in game {}, page {:?}, settings open {}, pictures {} from {} (shown {}, next {}, decoding {}), mods panel {:?}; entries: {}",
                menu.enabled,
                menu.up,
                menu.in_game,
                menu.page,
                world.resource::<crate::hud::HudState>().show_settings,
                p.paths.len(),
                if p.from.is_empty() { "nowhere" } else { p.from },
                p.current.is_some(),
                p.next.is_some(),
                p.job.is_some(),
                menu.mods_panel.map(|r| (r.min.x as i32, r.min.y as i32, r.max.x as i32, r.max.y as i32)),
                entries.join(", ")
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_folder_is_never_read() {
        let dir = std::env::temp_dir().join(format!("se_menu_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("export")).unwrap();
        std::fs::create_dir_all(dir.join("screenshots-hd")).unwrap();
        for f in ["a.jpg", "b.PNG", "c.txt", "export/sky_pos_x.png", "screenshots-hd/m.jpeg"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut found: Vec<String> = own_pictures(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        found.sort();
        assert_eq!(found, ["a.jpg", "b.PNG", "m.jpeg"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tall_or_broken_picture_is_skipped_and_a_big_one_shrinks() {
        let dir = std::env::temp_dir().join(format!("se_menu_decode_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, w: u32, h: u32| {
            let p = dir.join(name);
            image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255])).save(&p).unwrap();
            p
        };
        assert!(decode(&Pic::File(save("square.png", 64, 64))).is_none(), "a sky face");
        let wide = decode(&Pic::File(save("wide.png", 3440, 1440))).expect("a screenshot");
        assert!(wide.size[0] <= MAX_W as usize && wide.size[1] <= MAX_H as usize);
        assert!((wide.size[0] as f32 / wide.size[1] as f32 - 3440.0 / 1440.0).abs() < 0.01, "keeps its shape");
        std::fs::write(dir.join("broken.jpg"), b"not a jpeg").unwrap();
        assert!(decode(&Pic::File(dir.join("broken.jpg"))).is_none());
        // A JPEG decodes (the build's `image` has the jpeg feature).
        let j = dir.join("wide.jpg");
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(320, 180, image::Rgba([200, 100, 50, 255]))).to_rgb8().save(&j).unwrap();
        assert_eq!(decode(&Pic::File(j.clone())).map(|i| i.size), Some([320, 180]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The pack's backgrounds are listed and decode (what the menu shows when there are no screenshots).
    #[test]
    fn the_packs_backgrounds_decode() {
        let install = std::sync::Arc::new(super::super::assets::open_pack().expect("gmod.pak").unwrap());
        let pics = gmod_pictures(install);
        assert!(pics.len() >= 10, "{} pictures", pics.len());
        for pic in pics.iter().take(3) {
            let image = decode(pic).expect("a background decodes");
            assert!(image.size[0] <= MAX_W as usize && image.size[1] <= MAX_H as usize);
        }
    }
}
