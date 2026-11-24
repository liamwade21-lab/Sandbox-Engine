//! One of Elite's UI movies running (ARCHITECTURE.md A10): loaded from its converted SWF, sent the
//! game's messages, stepped on its own frame clock, drawn into the caller's texture, and read for
//! what it sends back.

use crate::bridge::{self, Interface, Link, Log, NoFiles, Ui};
use crate::frames::Frames;
use crate::gpu::Gpu;
use crate::host::Host;
use crate::target::Target;
use elite_ui::Value;
use ruffle_core::backend::navigator::NullExecutor;
use ruffle_core::backend::ui::FontDefinition;
use ruffle_core::external::Value as Ext;
use ruffle_core::font::{DefaultFont, FontFileData};
use ruffle_core::tag_utils::SwfMovieData;
use ruffle_core::{FloatDuration, Player, PlayerBuilder, PlayerMode};
use ruffle_render_wgpu::backend::WgpuRenderBackend;
use ruffle_render_wgpu::target::RenderTarget;
use ruffle_render_wgpu::wgpu;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

/// A font the movies' text fields can name, as a device font (Elite's font map: `$TitleFont`,
/// `$ParagraphFont`, ... and the fonts' own names).
#[derive(Clone)]
pub struct Font {
    pub name: String,
    pub bold: bool,
    pub italic: bool,
    /// The TrueType file.
    pub data: Arc<Vec<u8>>,
}

/// What a movie is made from.
pub struct MovieFiles {
    pub name: String,
    /// The converted SWF (`convert_movie`).
    pub swf: Vec<u8>,
    pub fonts: Vec<Font>,
    pub host: Arc<dyn Host>,
}

/// What a movie tells the game.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// `SendEventToGame(name, args)`.
    Event { name: String, args: Vec<Value> },
    /// `PlaySound(name, source)`.
    Sound { name: String, source: Option<String> },
    /// `DoAction(name, args)`; also `CapturingButtons`, `CapturingMouse` and `OnInputEvent` under
    /// their own names, with their arguments.
    Action { name: String, args: Vec<Value> },
    /// `UIOpened()`.
    Opened,
    /// `UIClosed()`.
    Closed,
    /// `SetReadyForInvokes()`: messages are delivered from now on.
    Ready,
    /// The movie's `trace`, `Trace`, `Break`, a script warning or an uncaught ActionScript error.
    Trace(String),
}

/// A running movie. Not `Send`: it lives on one thread.
pub struct Movie {
    player: Arc<Mutex<Player>>,
    executor: NullExecutor,
    link: Rc<Link>,
    /// Messages sent before the movie was ready, in order.
    sends: VecDeque<(String, Vec<Value>)>,
    /// DataStore writes made before the bridge was up, in order.
    stores: VecDeque<(Vec<String>, String, Value)>,
    /// Messages and writes refused because `WAITING_MOST` already waited.
    refused: usize,
    stage: (u32, u32),
    /// What the last render made of the movie as it is now.
    rendered: Rendered,
}

/// Whether the movie's frame as it is now was rendered (`Movie::changed`, `Movie::draw`).
#[derive(Clone, Copy, PartialEq)]
enum Rendered {
    /// Not since the movie was last stepped, sent a message or written to.
    Not,
    /// Held by `Frames` (`Frames::release`), and it differs from what the target shows.
    Held,
    /// The target shows it (drawn, or found the same as what was drawn).
    Shown,
}

/// The most messages (and, apart, DataStore writes) that wait for a movie to be ready: a movie that
/// never becomes ready cannot grow without end (the game sends tens a frame).
pub const WAITING_MOST: usize = 4096;

/// The name of the font map's paragraph font, used for text that names no font the movie has.
const PARAGRAPH_FONT: &str = "$ParagraphFont";

impl Movie {
    /// Loads a converted movie to draw into `target` (format `target::FORMAT`, sized as the caller
    /// wants: the movie is fitted to it by its own stage scale mode; the stage size is the plan).
    pub fn load(gpu: &Gpu, files: MovieFiles, target: wgpu::Texture) -> Result<Movie, String> {
        let url = format!("file:///movie/{}.swf", files.name);
        let data = SwfMovieData::from_data(&files.swf, url, None, None)
            .map_err(|e| format!("{}: not a movie: {e}", files.name))?;
        if !data.is_action_script_3() {
            return Err(format!("{}: not an ActionScript 3 movie (was it converted?)", files.name));
        }
        let stage = (data.width().to_pixels().round() as u32, data.height().to_pixels().round() as u32);
        let target = Target::new(target).map_err(|e| format!("{}: {e}", files.name))?;
        let (width, height) = (target.width(), target.height());
        let renderer = WgpuRenderBackend::new(gpu.descriptors.clone(), target)
            .map_err(|e| format!("{}: {e}", files.name))?;
        let executor = NullExecutor::new();
        let link = Rc::new(Link {
            movie: files.name.clone(),
            host: files.host,
            frame_rate: data.frame_rate().to_f64(),
            outbox: RefCell::new(Vec::new()),
            ready: Cell::new(false),
            bridge_up: Cell::new(false),
        });
        let player = PlayerBuilder::new()
            .with_renderer(Frames::new(renderer))
            .with_ui(Ui::new(link.clone()))
            .with_navigator(NoFiles::new(&executor))
            .with_log(Log(link.clone()))
            .with_external_interface(Box::new(Interface(link.clone())))
            // Debug: an uncaught ActionScript error is traced (an `Out::Trace`).
            .with_player_mode(PlayerMode::Debug)
            .with_movie(data)
            .with_viewport_dimensions(width, height, 1.0)
            .with_autoplay(true)
            .build();
        let fallback = link.host.get_mapped_font_name(PARAGRAPH_FONT);
        let movie = Movie {
            player,
            executor,
            link,
            sends: VecDeque::new(),
            stores: VecDeque::new(),
            refused: 0,
            stage,
            rendered: Rendered::Not,
        };
        {
            let mut player = movie.player();
            player.set_window_mode("transparent");
            for font in files.fonts {
                player.register_device_font(FontDefinition::FontFile {
                    name: font.name,
                    is_bold: font.bold,
                    is_italic: font.italic,
                    data: FontFileData::new_shared(font.data),
                    index: 0,
                });
            }
            // A STAND-IN (grade B: what Scaleform's font map does with a font name it does not know was
            // not read): text in a font the movie does not have (a TextField made by code and never
            // styled is Flash's Times New Roman) is drawn in the font map's paragraph font.
            for default in [
                DefaultFont::Sans,
                DefaultFont::Serif,
                DefaultFont::Typewriter,
                DefaultFont::JapaneseGothic,
                DefaultFont::JapaneseGothicMono,
                DefaultFont::JapaneseMincho,
            ] {
                player.set_default_font(default, vec![fallback.clone()]);
            }
        }
        Ok(movie)
    }

    fn player(&self) -> MutexGuard<'_, Player> {
        self.player.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The movie's stage size in pixels.
    pub fn stage(&self) -> (u32, u32) {
        self.stage
    }

    /// Whether the movie has called `SetReadyForInvokes`.
    pub fn ready(&self) -> bool {
        self.link.ready.get()
    }

    /// Sends a message to the movie's `HandleMessage` (before `ready` it waits, in order, up to
    /// `WAITING_MOST`; beyond that it is refused and an `Out::Trace` says so).
    pub fn send(&mut self, name: &str, args: &[Value]) {
        self.rendered = Rendered::Not;
        if self.ready() && self.sends.is_empty() {
            self.deliver(name, args);
        } else if self.sends.len() < WAITING_MOST {
            self.sends.push_back((name.to_string(), args.to_vec()));
        } else {
            self.refuse(&format!("message {name}"));
        }
    }

    /// Counts a refused message or write; the first one of a run is traced.
    fn refuse(&mut self, what: &str) {
        self.refused += 1;
        if self.refused == 1 {
            let movie = &self.link.movie;
            let text = format!("{movie} is not ready: {what} refused ({WAITING_MOST} already wait)");
            self.link.outbox.borrow_mut().push(Out::Trace(text));
        }
    }

    fn deliver(&self, name: &str, args: &[Value]) {
        let all = std::iter::once(Ext::String(name.to_string())).chain(args.iter().map(bridge::to_ext));
        self.player().call_internal_interface(bridge::HANDLE_MESSAGE, all);
    }

    /// Writes a property of a DataStore node (the node and its parents are made if they do not
    /// exist; listeners hear the change).
    pub fn store(&mut self, context: &[&str], property: &str, value: Value) {
        self.rendered = Rendered::Not;
        let context: Vec<String> = context.iter().map(|s| s.to_string()).collect();
        if self.link.bridge_up.get() && self.stores.is_empty() {
            self.write(&context, property, &value);
        } else if self.stores.len() < WAITING_MOST {
            self.stores.push_back((context, property.to_string(), value));
        } else {
            self.refuse(&format!("DataStore write {property}"));
        }
    }

    fn write(&self, context: &[String], property: &str, value: &Value) {
        let path = Ext::List(context.iter().cloned().map(Ext::String).collect());
        let args = [path, Ext::String(property.to_string()), bridge::to_ext(value)];
        self.player().call_internal_interface(bridge::STORE, args);
    }

    /// Runs the movie's own frames for `dt` seconds of game time (at its own frame rate). Returns
    /// whether anything may have changed, so `draw` is worth calling.
    pub fn step(&mut self, dt: f64) -> bool {
        self.rendered = Rendered::Not;
        self.executor.run();
        self.player().tick(FloatDuration::from_secs(dt));
        let delivered = self.flush();
        delivered || self.player().needs_render()
    }

    /// Delivers what waited for the movie to be ready or the bridge to be up.
    fn flush(&mut self) -> bool {
        let mut delivered = false;
        if self.link.bridge_up.get() {
            while let Some((context, property, value)) = self.stores.pop_front() {
                self.write(&context, &property, &value);
                delivered = true;
            }
        }
        if self.ready() {
            while let Some((name, args)) = self.sends.pop_front() {
                self.deliver(&name, &args);
                delivered = true;
            }
            if self.refused > 0 {
                let text = format!("{}: {} messages and writes were refused while it was not ready", self.link.movie, self.refused);
                self.link.outbox.borrow_mut().push(Out::Trace(text));
                self.refused = 0;
            }
        }
        delivered
    }

    /// Draws the movie into its target, transparent where it is empty. A frame that would draw
    /// exactly what the target already shows is not drawn (`frames.rs`): nothing reaches the GPU.
    pub fn draw(&mut self) {
        let skipping = {
            let mut player = self.player();
            let skipping = frames(&mut player).skipping();
            match self.rendered {
                Rendered::Held => frames(&mut player).draw_held(),
                Rendered::Shown if skipping => {}
                Rendered::Not | Rendered::Shown => player.render(),
            }
            skipping
        };
        self.rendered = if skipping { Rendered::Shown } else { Rendered::Not };
    }

    /// Whether `draw` would change the target now. It renders the frame (running the movie's
    /// `Event.RENDER` listeners, as `draw` would) and keeps it for `draw`, so asking costs no second
    /// render. The answer is always against the last frame really drawn into the target.
    ///
    /// Between `changed` and `draw` a caller may do anything (`step`, `send`, `store`, `changed`
    /// again, or not draw at all for as long as it likes): the kept frame is then out of date and
    /// `draw` renders anew, and nothing the kept frame had to draw is lost (`frames.rs`).
    pub fn changed(&mut self) -> bool {
        if self.rendered == Rendered::Not {
            let mut player = self.player();
            frames(&mut player).hold();
            player.render();
            let differs = frames(&mut player).release();
            drop(player);
            self.rendered = if differs { Rendered::Held } else { Rendered::Shown };
        }
        self.rendered == Rendered::Held
    }

    /// The frames drawn into the target since the movie was loaded.
    pub fn draws(&self) -> u64 {
        let player = self.player();
        let frames: &dyn Any = player.renderer();
        frames.downcast_ref::<Frames>().expect("the movie's renderer is Frames").drawn()
    }

    /// The movie's frame rate, from its header (frames a second).
    pub fn frame_rate(&self) -> f64 {
        self.link.frame_rate
    }

    /// Turns the skipping of unchanged frames off (every `draw` draws), for comparisons.
    #[cfg(test)]
    pub(crate) fn set_skipping(&mut self, skipping: bool) {
        frames(&mut self.player()).set_skipping(skipping);
        self.rendered = Rendered::Not;
    }

    /// The time spent testing frames (comparing, and keeping the drawn list) and the frames compared.
    #[cfg(test)]
    pub(crate) fn checking(&mut self) -> (std::time::Duration, u64) {
        frames(&mut self.player()).checking.get()
    }

    /// The bitmap-cache entries drawn so far.
    #[cfg(test)]
    pub(crate) fn caches_drawn(&mut self) -> u64 {
        frames(&mut self.player()).caches_drawn
    }

    /// What the movie has told the game since the last call.
    pub fn drain(&mut self) -> Vec<Out> {
        std::mem::take(&mut *self.link.outbox.borrow_mut())
    }
}

/// The player's renderer, which `Movie::load` made a `Frames`.
fn frames(player: &mut Player) -> &mut Frames {
    let renderer: &mut dyn Any = player.renderer_mut();
    renderer.downcast_mut::<Frames>().expect("the movie's renderer is Frames")
}
