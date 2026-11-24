//! The side panels' feeders and the focus machine (`elite_ui::feed::Panels`, `elite_ui::Focus`, task UI5a) proved on Elite's
//! own movies: the four panel movies are converted and run as `hud_feed.rs` runs the HUD's (its loading code is repeated
//! here: the proofs' rig is private to the crate's unit tests), the shipped focus machine (`ui/focus.json`) takes a script
//! of key presses, `Panels` feeds `PanelFacts`, what the movies send back goes through `panels::on_out` and `Panels::hear`
//! and the ship's actions change the facts as the mod would. The run fails on any ActionScript error, any error line in
//! Ruffle's log and any message a movie has no listener for; then it checks what came back (the sounds each key press
//! posts, against `re/elite/notes/ui_sounds.md`'s names, and the events). The text host is `elite_ui::Text` on the FULL
//! string table (the shipped `ui/strings/en.json`), its database answers `elite_ui::feed::database_item`. Pictures of every
//! page go to `.optimize/scratch/run/ui5a/`, with the log of what was sent and what came out.
//!
//! Ignored: it reads the UI export (`ELITE_UI_EXPORT`, default `re/elite/frea_export/sidewinder_ui/decoded`) and the mod's
//! shipped `ui/` files, and needs a Vulkan GPU.
//! `cargo test --profile agent -p elite-gfx --test panels_feed -- --ignored --nocapture`.

use elite_gfx::{Font, Gpu, Host, Movie, MovieFiles, Out, convert_movie, target};
use elite_ui::facts::location_type as kind;
use elite_ui::feed::{Movie as Which, Outbox, Panels, database_item};
use elite_ui::focus::{Focus, FocusOut};
use elite_ui::panel_facts::{
    FireGroups, FireWeapon, FlightPhase, Functions, Location, LogLine, Module, PanelFacts, Power, Ship,
};
use elite_ui::panels::{Function, PanelAction, on_out};
use elite_ui::{Text, Value};
use ruffle_render_wgpu::backend::{create_wgpu_instance, request_adapter_and_device};
use ruffle_render_wgpu::wgpu;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The game's frame: 60 Hz, as the proofs step.
const STEP: f64 = 1.0 / 60.0;
/// The panel movies, in the focus machine's order.
const PANELS: [Which; 4] = [Which::CockpitExternalPanel, Which::CockpitInternalPanel2, Which::CockpitChatPanel, Which::RolePanel3];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn export() -> PathBuf {
    std::env::var_os("ELITE_UI_EXPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace().join("re/elite/frea_export/sidewinder_ui/decoded"))
}

fn shipped(file: &str) -> PathBuf {
    workspace().join("crates/se_engine/assets/mods/sidewinder/ui").join(file)
}

/// The game's text host, with Elite's own text rules on the full table; its database is the facts' modules. Every
/// translated token is kept to look at.
struct TextHost {
    text: Text,
    fonts: HashMap<String, String>,
    facts: Mutex<PanelFacts>,
    seen: Mutex<Vec<(String, String)>>,
}

impl Host for TextHost {
    fn translate(&self, text: &str) -> String {
        let out = self.text.translate(text);
        if text.contains('$') {
            self.seen.lock().unwrap().push((text.to_string(), out.clone()));
        }
        out
    }

    fn get_string(&self, key: &str) -> String {
        self.text.get_string(key)
    }

    fn make_untranslated_string(&self, text: &str) -> String {
        text.to_string()
    }

    fn find_in_translation_string(&self, text: &str, find: &str, from: f64) -> f64 {
        let t = self.translate(text);
        let from = from.max(0.0) as usize;
        t.get(from..).and_then(|rest| rest.find(find)).map_or(-1.0, |i| (i + from) as f64)
    }

    fn localise_number(&self, number: f64, format: &str, extra: Option<&str>) -> String {
        self.text.localise_number(number, format, extra)
    }

    fn does_text_symbol_exist(&self, key: &str) -> bool {
        self.text.contains(key)
    }

    fn get_mapped_font_name(&self, name: &str) -> String {
        self.fonts.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    fn get_database_item_data(&self, id: u32) -> Value {
        database_item(&self.facts.lock().unwrap(), id).unwrap_or(Value::Null)
    }
}

/// The font library's faces and the font map's names, as `hud_feed.rs` makes them.
fn fonts(export: &Path) -> Vec<Font> {
    let json = |p: &str| -> serde_json::Value { serde_json::from_slice(&std::fs::read(export.join(p)).unwrap()).unwrap() };
    let mut library: Vec<(String, bool, Arc<Vec<u8>>)> = Vec::new();
    for font in json("fonts/fonts_roman.glyphs.json").as_array().unwrap() {
        let (name, bold) = (font["name"].as_str().unwrap(), font["bold"].as_bool().unwrap());
        if library.iter().any(|(n, b, _)| n == name && *b == bold) {
            continue;
        }
        let file = format!("{}{}__fonts_roman.ttf", name.replace(' ', "_"), if bold { "_Bold" } else { "" });
        library.push((name.to_string(), bold, Arc::new(std::fs::read(export.join("fonts").join(&file)).expect(&file))));
    }
    let face = |family: &str, bold: bool| {
        library
            .iter()
            .find(|(n, b, _)| n == family && *b == bold)
            .or_else(|| library.iter().find(|(n, b, _)| n.starts_with(family) && *b == bold))
            .map(|(_, _, d)| d.clone())
    };
    let mut out: Vec<Font> = library
        .iter()
        .map(|(name, bold, data)| Font { name: name.clone(), bold: *bold, italic: false, data: data.clone() })
        .collect();
    for entry in json("strings/fontmap.json")["entries"].as_array().unwrap() {
        let (name, family, bold) =
            (entry["name"].as_str().unwrap(), entry["font"].as_str().unwrap(), entry["bold"].as_bool().unwrap());
        let regular = face(family, bold).or_else(|| face(family, false)).expect("a font map face");
        let heavy = face(family, true).unwrap_or_else(|| regular.clone());
        out.push(Font { name: name.to_string(), bold: false, italic: false, data: regular });
        out.push(Font { name: name.to_string(), bold: true, italic: false, data: heavy });
    }
    out
}

struct Rig {
    gpu: Gpu,
    host: Arc<TextHost>,
    fonts: Vec<Font>,
    bridge: Vec<u8>,
    export: PathBuf,
}

impl Rig {
    fn new() -> Rig {
        let export = export();
        assert!(export.join("movies").is_dir(), "no UI export at {}", export.display());
        let backends = wgpu::Backends::VULKAN;
        let instance = create_wgpu_instance(backends, wgpu::BackendOptions::default(), None);
        let (adapter, device, queue) = futures::executor::block_on(request_adapter_and_device(
            backends,
            &instance,
            None,
            wgpu::PowerPreference::HighPerformance,
        ))
        .expect("a Vulkan device");
        let text = Text::from_json(&std::fs::read(shipped("strings/en.json")).unwrap()).expect("the full string table");
        let map: serde_json::Value =
            serde_json::from_slice(&std::fs::read(export.join("strings/fontmap.json")).unwrap()).unwrap();
        let fonts_map = map["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["name"].as_str().unwrap().to_string(), e["font"].as_str().unwrap().to_string()))
            .collect();
        let host = Arc::new(TextHost { text, fonts: fonts_map, facts: Mutex::new(PanelFacts::default()), seen: Mutex::new(Vec::new()) });
        let bridge = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("as3/bridge.abc")).unwrap();
        Rig { gpu: Gpu::new(instance, adapter, device, queue), host, fonts: fonts(&export), bridge, export }
    }

    /// The movie converted and loaded, stepped until it is ready, with its stage-sized target.
    fn load(&self, name: &str) -> (Movie, wgpu::Texture) {
        let source = std::fs::read(self.export.join(format!("movies/{name}/{name}.swf"))).unwrap();
        let swf = convert_movie(&source, &self.bridge).unwrap_or_else(|e| panic!("{name}: {e}"));
        let decompressed = ruffle_core::swf::decompress_swf(&swf[..]).unwrap();
        let header = decompressed.header.stage_size();
        let stage = (
            (header.x_max - header.x_min).to_pixels().round() as u32,
            (header.y_max - header.y_min).to_pixels().round() as u32,
        );
        let texture = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d { width: stage.0, height: stage.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let files = MovieFiles { name: name.to_string(), swf, fonts: self.fonts.clone(), host: self.host.clone() };
        let mut movie = Movie::load(&self.gpu, files, texture.clone()).unwrap_or_else(|e| panic!("{e}"));
        let mut t = 0.0;
        while !movie.ready() {
            assert!(t < 10.0, "{name}: not ready after 10 s");
            movie.step(STEP);
            t += STEP;
        }
        (movie, texture)
    }

    /// The texture as a PNG over dark grey (premultiplied colours over grey 24), to look at.
    fn shot(&self, texture: &wgpu::Texture, path: &Path) {
        let (w, h) = (texture.width(), texture.height());
        let padded = (w * 4).div_ceil(256) * 256;
        let device = self.gpu.device();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            texture.size(),
        );
        self.gpu.queue().submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let data = slice.get_mapped_range().expect("mapped range");
        let mut view = image::RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let at = (y * padded + x * 4) as usize;
                let p = &data[at..at + 4];
                let a = u32::from(p[3]);
                let over = |c: u8| (u32::from(c) + 24 * (255 - a) / 255).min(255) as u8;
                view.put_pixel(x, y, image::Rgb([over(p[0]), over(p[1]), over(p[2])]));
            }
        }
        drop(data);
        view.save(path).unwrap();
    }
}

/// A trace that is an ActionScript error or a message with no listener (as `hud_feed.rs` reads them).
fn is_error(text: &str) -> bool {
    text.starts_with("Break:")
        || text.contains("Error #")
        || text.contains("no listener for message")
        || ["TypeError", "ReferenceError", "ArgumentError", "RangeError", "VerifyError", "SecurityError"]
            .iter()
            .any(|e| text.contains(e))
        || text.starts_with("tween onComplete:")
}

/// The stock Sidewinder as the mod has it (the modules and numbers of `elite-sim`'s `data.rs` and `panels::MODULES`: the
/// journal's loadout), in a system with a star, a planet, its moon and a neighbouring system, flying.
fn facts() -> PanelFacts {
    let place = |id, name: &str, kind, distance_m, parent| Location { id, name: name.into(), kind, distance_m, parent, ..Location::default() };
    let module = |id, item: &str, kind, size, rating, power_mw, switchable| Module {
        id,
        item: item.into(),
        kind,
        size,
        rating,
        power_mw,
        on: true,
        switchable,
        health: 1.0,
        priority: None,
        weapon_mode: None,
    };
    let mut laser = module(9, "Hpt_PulseLaser_Gimbal_Small", 7, 1, 'G', 0.39, true);
    laser.weapon_mode = Some(1);
    let mut laser2 = laser.clone();
    laser2.id = 10;
    let mut star = place(1, "Ross 128", kind::STAR, 2.9e11 * 0.299_792_458, None);
    star.current = true;
    let mut neighbour = place(20, "Wolf 359", kind::STAR, 3.8e16, None);
    neighbour.other_system = true;
    let mut far = place(21, "Sol", kind::STAR, 1.0e17, None);
    far.other_system = true;
    PanelFacts {
        system: "Ross 128".into(),
        locations: vec![
            star,
            place(2, "Ross 128 b", kind::LANDABLE_PLANET, 2.29e11 * 0.299_792_458, Some(1)),
            place(3, "Ross 128 b A", kind::LANDABLE_PLANET, 2.28e11 * 0.299_792_458, Some(2)),
            place(4, "Ross 128 c", kind::PLANET, 4.5e11 * 0.299_792_458, Some(1)),
            neighbour,
            far,
        ],
        commander: None,
        ship: Ship { model: "sidewinder".into(), name: None, ident: None },
        modules: vec![
            module(1, "Int_PowerPlant_Size2_Class1", 10, 2, 'E', 0.0, false),
            module(2, "Int_Engine_Size2_Class1", 22, 2, 'E', 2.0, true),
            module(3, "Int_Hyperdrive_Size2_Class1", 9, 2, 'E', 0.16, true),
            module(4, "Int_LifeSupport_Size1_Class1", 23, 1, 'E', 0.32, true),
            module(5, "Int_PowerDistributor_Size1_Class1", 20, 1, 'E', 0.32, false),
            module(6, "Int_Sensors_Size1_Class1", 6, 1, 'E', 0.16, true),
            module(7, "Int_FuelTank_Size1_Class3", 11, 1, 'C', 0.0, false),
            module(8, "Int_ShieldGenerator_Size2_Class1", 4, 2, 'E', 0.9, true),
            laser,
            laser2,
        ],
        power: Power { capacity_mw: 6.4, available_mw: 6.4 },
        fire_groups: FireGroups {
            count: 1,
            active: 0,
            weapons: vec![
                FireWeapon { module: 9, slot: 0, trigger: 0, weapon_type: 0, bindings: vec![1] },
                FireWeapon { module: 10, slot: 1, trigger: 0, weapon_type: 0, bindings: vec![1] },
            ],
        },
        cargo_capacity_t: Some(4.0),
        functions: Functions { flight_assist: true, ..Functions::default() },
        flight: FlightPhase::Normal,
        leave_allowed: false,
        comms: vec![LogLine { id: 1, text: "Proof run: comms line".into(), time_of_day_s: 45_000.0 }],
        time_of_day_s: Some(45_000.0),
    }
}

/// What the game does with a ship action in this proof (as the mod's `fly` would: the lock, the module switches, the
/// functions; the drive and the seat are not flown here).
fn apply(facts: &mut PanelFacts, action: &PanelAction) {
    match action {
        PanelAction::LockDestination(id)
        | PanelAction::EngageSupercruise { lock: Some(id) }
        | PanelAction::EngageHyperdrive { lock: Some(id) } => facts.locations.iter_mut().for_each(|l| l.locked = l.id == *id),
        PanelAction::Unlock => facts.locations.iter_mut().for_each(|l| l.locked = false),
        PanelAction::SetModule { id, on } => {
            if let Some(m) = facts.modules.iter_mut().find(|m| m.id == *id) {
                m.on = *on;
            }
        }
        PanelAction::Toggle(f) => {
            let s = &mut facts.functions;
            match f {
                Function::FlightAssist => s.flight_assist ^= true,
                Function::Lights => s.lights ^= true,
                Function::LandingGear => s.gear_down ^= true,
                Function::CargoScoop => s.cargo_scoop ^= true,
                Function::SilentRunning => s.silent_running ^= true,
            }
        }
        _ => {}
    }
}

/// One step of the script.
#[derive(Clone, Copy, Debug)]
enum Do {
    Seat,
    /// A key: pressed now, released a tenth of a second later.
    Key(&'static str),
    /// `UIFocus` pressed or released alone (the held key).
    Hold(bool),
    Landed,
    Shot(&'static str),
    /// The focus machine must be in this state.
    Expect(&'static str),
    /// The cockpit must be flying: no panel focused, the flight keys live (the modal set off), no head look.
    Flying,
}

/// The script: (seconds, step). Pictures are taken a second or more after the key that brought the page up.
fn script() -> Vec<(f64, Do)> {
    use Do::*;
    let mut s: Vec<(f64, Do)> = Vec::new();
    let mut t = 0.0;
    let mut at = |dt: f64, d: Do| {
        t += dt;
        s.push((t, d));
    };
    at(0.0, Seat);
    at(1.5, Shot("flight"));
    at(0.0, Flying);
    at(0.1, Key("FocusLeftPanel"));
    at(0.1, Expect("External Panel"));
    at(1.5, Shot("left_navigation"));
    at(0.2, Key("UI_Down"));
    at(0.4, Key("UI_Down"));
    at(0.8, Shot("left_navigation_moved"));
    // The third row (Ross 128 b A): its pop-up, Lock destination.
    at(0.2, Key("UI_Select"));
    at(1.2, Shot("left_popup"));
    at(0.2, Key("UI_Select"));
    at(1.2, Shot("left_locked"));
    // Locked: its pop-up has Unlock and Engage supercruise; UI_Back is the pop-up's (the panel stays).
    at(0.2, Key("UI_Select"));
    at(1.2, Shot("left_popup_locked"));
    at(0.2, Key("UI_Back"));
    at(0.1, Expect("External Panel"));
    at(1.1, Shot("left_popup_back"));
    at(0.2, Key("UI_Select"));
    at(1.0, Key("UI_Select"));
    at(1.2, Shot("left_unlocked"));
    // Unlocked: "Lock and supercruise", the pop-up's second button.
    at(0.2, Key("UI_Select"));
    at(1.0, Key("UI_Right"));
    at(0.6, Shot("left_popup_engage"));
    at(0.2, Key("UI_Select"));
    at(1.2, Shot("left_engaged"));
    at(0.2, Key("CycleNextPanel"));
    at(1.2, Shot("left_transactions"));
    at(0.2, Key("CycleNextPanel"));
    at(1.2, Shot("left_contacts"));
    // Target is disabled with no contact targeted (as in Elite): the next tab is the Summary, then Navigation again.
    at(0.2, Key("CycleNextPanel"));
    at(1.2, Shot("left_summary"));
    at(0.2, Key("CycleNextPanel"));
    at(1.2, Shot("left_wrapped"));
    // The panel's own key again closes it; UIFocus pressed inside the panel leaves it, its release does nothing more.
    at(0.2, Key("FocusLeftPanel"));
    at(0.1, Expect("Back From"));
    at(0.0, Flying);
    at(0.3, Shot("left_closing"));
    at(0.6, Key("FocusLeftPanel"));
    at(0.1, Expect("External Panel"));
    at(1.2, Shot("left_reopened"));
    at(0.2, Hold(true));
    at(0.1, Expect("Back From"));
    at(0.3, Hold(false));
    at(0.1, Expect("Back From"));
    at(0.0, Flying);
    at(0.2, Key("FocusRightPanel"));
    at(0.1, Expect("Internal Panel"));
    at(1.5, Shot("right_home"));
    at(0.2, Key("CycleNextPanel"));
    at(1.5, Shot("right_modules"));
    // The first row (the thrusters, the highest draw) selected: its pop-up, the power switch off, then on again.
    at(0.2, Key("UI_Select"));
    at(1.0, Shot("right_module_popup"));
    at(0.2, Key("UI_Select"));
    at(1.2, Shot("right_module_off"));
    at(0.2, Key("UI_Select"));
    at(0.8, Key("UI_Select"));
    at(1.2, Shot("right_module_on"));
    at(0.2, Key("CycleNextPanel"));
    at(1.2, Shot("right_fire_groups"));
    at(0.2, Key("CycleNextPanel"));
    at(1.2, Shot("right_ship_functions"));
    // Into the Functions list, down its left column (lights, gear, scoop) and over to silent running; select it.
    at(0.2, Key("UI_Right"));
    at(0.4, Key("UI_Down"));
    at(0.4, Key("UI_Down"));
    at(0.4, Key("UI_Right"));
    at(0.6, Key("UI_Select"));
    at(1.2, Shot("right_ship_functions_toggled"));
    at(0.2, Key("UI_Select"));
    at(0.8, Shot("right_ship_functions_back"));
    // UI_Back with no pop-up leaves the panel (through Select UI Mode, left at once with UIFocus up); a UIFocus tap after.
    at(0.2, Key("UI_Back"));
    at(0.1, Expect("Back From"));
    at(0.0, Flying);
    at(0.4, Hold(true));
    at(0.1, Expect("Select UI Mode"));
    at(0.2, Hold(false));
    at(0.1, Expect("Back From"));
    at(0.0, Flying);
    at(0.4, Key("FocusRightPanel"));
    at(0.1, Expect("Internal Panel"));
    at(0.5, Key("CycleNextPanel"));
    at(1.2, Shot("right_inventory"));
    // Storage is disabled (no on-foot locker): the next tab is Status.
    at(0.2, Key("CycleNextPanel"));
    at(1.2, Shot("right_status"));
    at(0.2, Key("FocusRightPanel"));
    at(0.1, Expect("Back From"));
    at(0.0, Flying);
    at(0.4, Key("FocusRadarPanel"));
    at(0.1, Expect("Role Switch"));
    at(1.5, Shot("role_commander_in_flight"));
    at(0.2, Key("UI_Down"));
    at(1.2, Shot("role_fighter"));
    at(0.2, Key("UI_Down"));
    at(1.2, Shot("role_srv"));
    at(0.2, Key("UI_Up"));
    at(0.3, Key("UI_Up"));
    at(0.2, Landed);
    at(1.2, Shot("role_commander_landed"));
    at(0.2, Key("FocusRadarPanel"));
    at(0.1, Expect("Back From"));
    at(0.0, Flying);
    at(0.4, Key("FocusCommsPanel"));
    at(0.1, Expect("Comms Panel Full"));
    at(1.5, Shot("comms_full"));
    at(0.2, Key("FocusCommsPanel"));
    at(0.1, Expect("Back From"));
    at(0.4, Key("QuickCommsPanel"));
    at(0.1, Expect("Comms Panel Mini"));
    at(1.2, Shot("comms_mini"));
    at(0.2, Key("FocusCommsPanel"));
    at(0.1, Expect("Back From"));
    // UIFocus held: alone, then with a direction.
    at(0.3, Hold(true));
    at(0.1, Expect("Select UI Mode"));
    at(0.3, Hold(false));
    at(0.1, Expect("Back From"));
    at(0.3, Hold(true));
    at(0.1, Expect("Select UI Mode"));
    at(0.2, Key("UI_Left"));
    at(0.1, Expect("External Panel"));
    // The release keeps the panel the direction opened (the hold rule); its own key closes it.
    at(0.3, Hold(false));
    at(0.1, Expect("External Panel"));
    at(0.4, Key("FocusLeftPanel"));
    at(0.1, Expect("Back From"));
    at(0.0, Flying);
    at(1.5, Shot("flight_after"));
    // Landed: the role panel's Disembark; the game leaves the seat, which hides the panels; then he sits again.
    at(0.2, Key("FocusRadarPanel"));
    at(0.1, Expect("Role Switch"));
    at(1.2, Key("UI_Right"));
    at(0.6, Shot("role_disembark"));
    at(0.2, Key("UI_Select"));
    at(0.5, Expect("Idle"));
    at(1.5, Shot("seat_left"));
    at(0.2, Seat);
    at(0.1, Expect("In Flight"));
    at(1.5, Shot("seat_again"));
    // The role panel's Disembark disabled its input (`InputManager.instance.disable()`): its FocusOn gives the keys back.
    at(0.2, Key("FocusRadarPanel"));
    at(0.1, Expect("Role Switch"));
    at(1.0, Key("UI_Left"));
    at(0.4, Key("UI_Down"));
    at(1.2, Shot("role_seat_again"));
    s
}

/// The `$KEY;` and `$KEY:` tokens left in a text.
fn raw_keys(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in text.split('$').skip(1) {
        let key: String = piece.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if !key.is_empty() && matches!(piece[key.len()..].chars().next(), Some(';' | ':')) {
            out.push(key);
        }
    }
    out
}

/// What the run gave: the log, the errors, the messages sent (by name), what came back from each movie ("sound NAME",
/// "event NAME ARGS..."), and the ship's actions.
#[derive(Default)]
struct Run {
    log: String,
    errors: Vec<String>,
    sent: HashMap<&'static str, usize>,
    back: Vec<(Which, String)>,
    actions: Vec<PanelAction>,
    /// `Show` and `Hide` sent, per movie.
    life: HashMap<(Which, &'static str), usize>,
}

fn panel_index(movie: Which) -> usize {
    PANELS.iter().position(|m| *m == movie).expect("a panel movie")
}

impl Run {
    fn deliver(&mut self, movies: &mut [(Movie, wgpu::Texture)], movie: Which, name: &str, args: &[Value], t: f64) {
        let _ = writeln!(self.log, "{t:7.3} SEND {} {name} {args:?}", movie.name());
        if matches!(name, "Show" | "Hide") {
            *self.life.entry((movie, if name == "Show" { "Show" } else { "Hide" })).or_default() += 1;
        }
        movies[panel_index(movie)].0.send(name, args);
    }

    fn focus_out(&mut self, movies: &mut [(Movie, wgpu::Texture)], outs: Vec<FocusOut>, t: f64) {
        for o in outs {
            let m = o.msg();
            *self.sent.entry(m.name).or_default() += 1;
            self.deliver(movies, m.movie, m.name, &m.args, t);
        }
    }

    /// One frame: the facts fed, the stores then the messages to their movies, every movie stepped, its answers heard (by
    /// the focus machine too: its pop-ups); a Disembark leaves the seat, as the game would.
    fn frame(&mut self, panels: &mut Panels, focus: &mut Focus, movies: &mut [(Movie, wgpu::Texture)], facts: &mut PanelFacts, t: f64) {
        let mut seat_outs = Vec::new();
        let mut outbox = Outbox::default();
        panels.feed(facts, &focus.state(), STEP, &mut outbox);
        for s in &outbox.stores {
            let _ = writeln!(self.log, "{t:7.3} STORE {} {:?} {} {:?}", s.movie.name(), s.context, s.property, s.value);
            let context: Vec<&str> = s.context.iter().map(String::as_str).collect();
            movies[panel_index(s.movie)].0.store(&context, &s.property, s.value.clone());
        }
        for msg in &outbox.msgs {
            *self.sent.entry(msg.name).or_default() += 1;
            self.deliver(movies, msg.movie, msg.name, &msg.args, t);
        }
        for (k, (movie, _)) in movies.iter_mut().enumerate() {
            if movie.step(STEP) {
                movie.draw();
            }
            for o in movie.drain() {
                let which = PANELS[k];
                let _ = writeln!(self.log, "{t:7.3} OUT {} {o:?}", which.name());
                match &o {
                    Out::Trace(text) if is_error(text) => self.errors.push(format!("{t:.3} {}: {text}", which.name())),
                    Out::Sound { name, .. } => self.back.push((which, format!("sound {name}"))),
                    Out::Event { name, args } => {
                        let words: Vec<String> = args
                            .iter()
                            .map(|a| match a {
                                Value::Str(s) => s.clone(),
                                Value::Bool(b) => b.to_string(),
                                Value::Num(n) => n.to_string(),
                                other => format!("{other:?}"),
                            })
                            .collect();
                        self.back.push((which, format!("event {name} {}", words.join(" ")).trim_end().to_string()));
                        let acts = on_out(which, name, args);
                        focus.hear(&acts);
                        for action in panels.hear(&acts, facts) {
                            let _ = writeln!(self.log, "{t:7.3} ACTION {action:?}");
                            apply(facts, &action);
                            if action == PanelAction::Disembark && facts.leave_allowed {
                                seat_outs.extend(focus.seat(false));
                            }
                            self.actions.push(action);
                        }
                    }
                    _ => {}
                }
            }
        }
        self.focus_out(movies, seat_outs, t);
    }
}

#[test]
#[ignore = "reads the UI export (out of git) and the mod's shipped ui files, and needs a Vulkan GPU"]
fn panels_on_the_real_movies() {
    let out_dir = workspace().join(".optimize/scratch/run/ui5a");
    std::fs::create_dir_all(&out_dir).unwrap();
    let ruffle_path = out_dir.join("panels_feed.ruffle.log");
    let ruffle_log = Mutex::new(std::fs::File::create(&ruffle_path).unwrap());
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .with_writer(ruffle_log)
        .finish();
    let rig = Rig::new();
    let mut run = Run::default();
    let mut states = Vec::new();
    tracing::subscriber::with_default(subscriber, || {
        let mut movies: Vec<(Movie, wgpu::Texture)> = PANELS.iter().map(|m| rig.load(m.name())).collect();
        // The detector's own check: a message no movie listens for must come back as an error trace.
        movies[0].0.send("NoSuchMessageUi5a", &[]);
        movies[0].0.step(STEP);
        let caught = movies[0].0.drain().iter().any(|o| matches!(o, Out::Trace(t) if is_error(t)));
        assert!(caught, "a message without a listener is seen (the detector works)");
        let mut focus = Focus::from_json(&std::fs::read(shipped("focus.json")).unwrap()).expect("the shipped focus machine");
        let mut panels = Panels::default();
        let mut facts = facts();
        let steps = script();
        let end = steps.last().unwrap().0 + 0.5;
        let mut next = 0;
        let mut releases: Vec<(f64, &'static str)> = Vec::new();
        let frames = (end / STEP).round() as u64;
        for frame in 0..=frames {
            let t = frame as f64 * STEP;
            while next < steps.len() && steps[next].0 <= t + 1e-9 {
                let (_, step) = steps[next];
                next += 1;
                let _ = writeln!(run.log, "{t:7.3} DO {step:?}");
                match step {
                    Do::Seat => {
                        let outs = focus.seat(true);
                        run.focus_out(&mut movies, outs, t);
                    }
                    Do::Key(k) => {
                        let outs = focus.input(k, true);
                        run.focus_out(&mut movies, outs, t);
                        releases.push((t + 0.1, k));
                    }
                    Do::Hold(down) => {
                        let outs = focus.input("UIFocus", down);
                        run.focus_out(&mut movies, outs, t);
                    }
                    Do::Landed => {
                        facts.flight = FlightPhase::Landed;
                        facts.leave_allowed = true;
                        facts.functions.gear_down = true;
                        facts.functions.gear_locked = true;
                    }
                    Do::Shot(name) => {
                        for (k, (movie, texture)) in movies.iter_mut().enumerate() {
                            movie.draw();
                            rig.shot(texture, &out_dir.join(format!("{name}_{}.png", PANELS[k].name())));
                        }
                    }
                    Do::Expect(state) => states.push((t, state.to_string(), focus.state().state)),
                    Do::Flying => {
                        let s = focus.state();
                        let flying = s.focused.is_none() && !s.quiet_flight && s.head.is_none();
                        let got = if flying { "flying".to_string() } else { format!("{s:?}") };
                        states.push((t, "flying".to_string(), got));
                    }
                }
            }
            releases.retain(|(at, k)| {
                if *at <= t + 1e-9 {
                    let outs = focus.input(k, false);
                    run.focus_out(&mut movies, outs, t);
                    false
                } else {
                    true
                }
            });
            *rig.host.facts.lock().unwrap() = facts.clone();
            focus.step(STEP);
            run.frame(&mut panels, &mut focus, &mut movies, &mut facts, t);
        }
    });
    let ruffle = std::fs::read_to_string(&ruffle_path).unwrap_or_default();
    for line in ruffle.lines().filter(|l| l.contains("ERROR")) {
        run.errors.push(format!("ruffle: {line}"));
    }
    let seen = rig.host.seen.lock().unwrap().clone();
    let _ = writeln!(run.log, "messages sent: {:?}", run.sent);
    for (from, to) in &seen {
        let _ = writeln!(run.log, "TRANSLATED {from:?} -> {to:?}");
    }
    std::fs::write(out_dir.join("panels_feed.log"), &run.log).unwrap();
    for (t, want, got) in &states {
        assert_eq!(got, want, "the focus machine's state at {t:.2} s");
    }
    assert!(run.errors.is_empty(), "errors (see panels_feed.log):\n{}", run.errors.join("\n"));
    // No `$KEY;` (or `$KEY:`) is left in a translated text where the full table has the key (a font face's `$TitleFont'`
    // is no token).
    let raw: Vec<&(String, String)> = seen.iter().filter(|(_, to)| raw_keys(to).iter().any(|k| rig.host.text.contains(k))).collect();
    assert!(raw.is_empty(), "a raw $KEY; shown where the full table has a string: {raw:?}");
    for name in ["Open", "SetPanelsMode", "SetDestinationData", "SetContactList", "ShowLocationPopup", "HidePopup", "SetPlayerInfo", "SetShipOrVehicleInfo", "SetModuleContext", "ShowModulePopup", "HideModulePopup", "SetFireGroups", "SetActiveFireGroup", "SetOptionsList", "UpdateOption", "SetTabData", "InitDataStore", "UpdateCommChannelList", "PushMessageToLog", "Show", "Hide", "FocusOn", "FocusOff", "OnInputButton"] {
        assert!(run.sent.contains_key(name), "{name} was sent in the run");
    }
    // The ship's actions that came back: the lock, the unlock, lock and supercruise on the same place, the thrusters off
    // and on, silent running, the disembark. (CancelJump has no button to press: the Navigation page's jump box shows only
    // while the drive counts down, which `PanelFacts` does not carry.)
    let locked = run.actions.iter().find_map(|a| match a {
        PanelAction::LockDestination(id) => Some(*id),
        _ => None,
    });
    let locked = locked.unwrap_or_else(|| panic!("a lock came back: {:?}", run.actions));
    let want = [
        PanelAction::Unlock,
        PanelAction::EngageSupercruise { lock: Some(locked) },
        PanelAction::SetModule { id: 2, on: false },
        PanelAction::SetModule { id: 2, on: true },
        PanelAction::Toggle(Function::SilentRunning),
        PanelAction::Disembark,
    ];
    assert_eq!(run.actions.iter().filter(|a| **a == PanelAction::Toggle(Function::SilentRunning)).count(), 2, "on and off");
    for w in &want {
        assert!(run.actions.contains(w), "{w:?} came back (actions: {:?})", run.actions);
    }
    // What came back, against the notes: the movies' own sounds and events for what was sent and pressed.
    let back = |movie: Which, what: &str| run.back.iter().any(|(m, w)| *m == movie && w.starts_with(what));
    let left = Which::CockpitExternalPanel;
    let right = Which::CockpitInternalPanel2;
    for (movie, what, why) in [
        (left, "sound showBegin", "Show (Content.show: AudioEvents.SHOW_BEGIN)"),
        (left, "event ShowComplete", "Show"),
        (left, "event ExternalOnPanel 1", "Show: the cycler re-sends the current tab"),
        (left, "sound focusChanged", "UI_Down on the Navigation list"),
        (left, "event LocationSelected", "UI_Select on a row"),
        (left, "sound popUpShowBegin", "ShowLocationPopup"),
        (left, "event OnLocationPopupButtonSelect 0", "UI_Select on the pop-up's first button, Lock destination"),
        (left, "event OnLocationPopupButtonSelect 1", "UI_Select on Unlock destination"),
        (left, "event OnLocationPopupButtonSelect 5", "UI_Right then UI_Select: Lock and supercruise"),
        (left, "event OnLocationPopupBack", "UI_Back with the pop-up open (InfoPopup.handleInput: CANCEL)"),
        (left, "event HideComplete", "Hide on leaving the seat"),
        (right, "event HideComplete", "Hide on leaving the seat"),
        (Which::RolePanel3, "event HidingPanel", "Hide on leaving the seat"),
        (Which::RolePanel3, "event OnDisembarkPressed", "UI_Select on Disembark, landed"),
        (left, "sound tabChanged", "CycleNextPanel"),
        (left, "event ExternalOnPanel 3", "the Contacts tab"),
        (right, "event InternalOnPanel 2", "the Modules tab"),
        (right, "event ToggleModuleState 2", "UI_Select on the thrusters' row"),
        (right, "event InternalOnModulePopup 1", "ShowModulePopup"),
        (right, "event InternalModuleOptionPopupRadioSelected 0", "UI_Select on the power switch"),
        (right, "event ComboOptionSelected 15", "UI_Select on silent running"),
        (Which::CockpitChatPanel, "event ChatPanelTabsReady", "Show"),
        (Which::RolePanel3, "sound Open_CockpitRoleSwitch", "Show"),
        (Which::RolePanel3, "event ShowingPanel", "Show"),
    ] {
        assert!(back(movie, what), "{} did not answer {what} ({why})", movie.name());
    }
    // Each side panel is shown when it gains the keys and hidden when it loses them, every time: its opening sound for
    // every `Show`, its closing sound and its `HideComplete` (the role panel's `HidingPanel`) for every `Hide`.
    let count = |movie: Which, what: &str| run.back.iter().filter(|(m, w)| *m == movie && w == what).count();
    for (movie, open, close, done) in [
        (left, "sound showBegin", "sound hide", "event HideComplete"),
        (right, "sound showBegin", "sound hide", "event HideComplete"),
        (Which::RolePanel3, "sound Open_CockpitRoleSwitch", "sound Close_CockpitRoleSwitch", "event HidingPanel"),
    ] {
        let shows = run.life.get(&(movie, "Show")).copied().unwrap_or(0);
        let hides = run.life.get(&(movie, "Hide")).copied().unwrap_or(0);
        assert!(shows >= 2 && hides >= 2, "{}: shown {shows}, hidden {hides} times", movie.name());
        assert_eq!(count(movie, open), shows, "{}: an opening sound for every Show", movie.name());
        assert_eq!(count(movie, close), hides, "{}: a closing sound for every Hide", movie.name());
        assert_eq!(count(movie, done), hides, "{}: {done} for every Hide", movie.name());
    }
    println!("pictures and log in {}", out_dir.display());
}
