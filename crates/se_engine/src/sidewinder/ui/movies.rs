//! The movies running (ARCHITECTURE.md A10, "Running the movies in the game (UI3)"): a main-thread-only resource
//! (`UiMovies`, the movies are not `Send`) owning the runtime's GPU objects (`elite_gfx::Gpu`, made from clones of the
//! engine's own instance, adapter, device and queue) and each movie.
//!
//! - Each movie's target is a Bevy `Image` made once at its stage size (`target_image`); the render world hands that image's
//!   texture back once it exists (`Handoff`, `hand_back`), and only then is the movie loaded to draw into it. Nothing is
//!   swapped later: a material's bind group keeps pointing at the image.
//! - At most one movie is loaded a frame (`step`), so boarding does not hitch on all of them at once.
//! - Each frame while seated: the frame's `UiMessages` go to their movies in order (a movie not loaded yet keeps them, then
//!   the movie itself until it is ready), every wanted movie steps by the frame's real seconds, draws only when its step or a
//!   message may have changed something, and what it sent goes to `UiReplies` (its sounds to `UiSounds` under its interface's
//!   name, an error trace logged once). A movie no longer wanted stops stepping and drawing and stays loaded.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{Extent3d, Texture, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor};
use bevy::render::renderer::{RenderAdapter, RenderDevice, RenderInstance, RenderQueue};
use bevy::render::texture::GpuImage;
use elite_gfx::{Font, Gpu, Host, Movie, MovieFiles, Out};
use elite_ui::feed::Movie as Which;
use elite_ui::Value;

use super::super::sound::ui::UiSounds;
use super::{interface, is_error, UiGrace, UiMessages, UiMovieState, UiReplies, UiWanted};

/// What the render world hands the main world: the runtime's GPU objects (made there, where the engine's instance is) and
/// each wanted image's texture, once it exists on the GPU.
#[derive(Default)]
struct HandoffState {
    want_gpu: bool,
    /// The GPU objects were made (once: the main world takes them).
    gpu_made: bool,
    gpu: Option<Gpu>,
    wanted: Vec<AssetId<Image>>,
    textures: HashMap<AssetId<Image>, Texture>,
}

/// Shared by both worlds: whether the render world has anything to do, and what it hands back.
#[derive(Resource, Clone, Default)]
pub struct Handoff(Arc<(AtomicBool, Mutex<HandoffState>)>);

impl Handoff {
    fn state(&self) -> std::sync::MutexGuard<'_, HandoffState> {
        self.0.1.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Asks for the GPU objects and for `image`'s texture.
    pub fn want(&self, image: AssetId<Image>) {
        let mut s = self.state();
        s.want_gpu = true;
        if !s.wanted.contains(&image) && !s.textures.contains_key(&image) {
            s.wanted.push(image);
        }
        self.0.0.store(true, Ordering::Relaxed);
    }

    fn take_gpu(&self) -> Option<Gpu> {
        self.state().gpu.take()
    }

    fn texture(&self, image: AssetId<Image>) -> Option<Texture> {
        self.state().textures.get(&image).cloned()
    }

    /// Forgets every texture (the images are going) and the GPU objects (the movies holding them are going: a new ship's
    /// movies get new ones).
    pub fn clear(&self) {
        let mut s = self.state();
        s.wanted.clear();
        s.textures.clear();
        s.gpu = None;
        s.gpu_made = false;
        s.want_gpu = false;
    }
}

/// The render world, while the main world waits for something: the GPU objects once, and each wanted image's texture.
pub fn hand_back(handoff: Res<Handoff>, images: Res<RenderAssets<GpuImage>>, instance: Res<RenderInstance>, adapter: Res<RenderAdapter>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let mut s = handoff.state();
    if s.want_gpu && !s.gpu_made {
        let t0 = Instant::now();
        s.gpu = Some(Gpu::new((**instance).clone(), (**adapter).clone(), device.wgpu_device().clone(), (**queue).clone()));
        s.gpu_made = true;
        info!("sidewinder ui: the movies' GPU objects are made ({:.1} ms)", t0.elapsed().as_secs_f64() * 1e3);
    }
    let waiting = std::mem::take(&mut s.wanted);
    for id in waiting {
        match images.get(id) {
            Some(g) => {
                s.textures.insert(id, g.texture.clone());
            }
            None => s.wanted.push(id),
        }
    }
    let busy = (s.want_gpu && !s.gpu_made) || !s.wanted.is_empty();
    handoff.0.0.store(busy, Ordering::Relaxed);
}

/// The render world has something to do.
pub fn handoff_busy(handoff: Res<Handoff>) -> bool {
    handoff.0.0.load(Ordering::Relaxed)
}

/// A movie's target: its stage size, `elite_gfx::target::FORMAT`, transparent, drawn into by the movie and sampled by the
/// materials as stored (sRGB-encoded, premultiplied). A screen movie's (`srgb_view`) is sampled through an sRGB view, so
/// the screen's painter, which takes a texture's samples as linear, shows its texels as they are.
pub fn target_image(stage: [u32; 2], srgb_view: bool) -> Image {
    let size = Extent3d { width: stage[0].max(1), height: stage[1].max(1), depth_or_array_layers: 1 };
    let mut image = Image::new_fill(size, TextureDimension::D2, &[0, 0, 0, 0], TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC;
    if srgb_view {
        image.texture_descriptor.view_formats = &[TextureFormat::Rgba8UnormSrgb];
        image.texture_view_descriptor = Some(TextureViewDescriptor { format: Some(TextureFormat::Rgba8UnormSrgb), ..default() });
    }
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// A movie's cost: its last frame's step and draw (ms), their mean over the frames it ran, its load.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cost {
    pub last_ms: f64,
    pub total_ms: f64,
    pub frames: u64,
    /// Frames drawn into its target (`Movie::draws`: the runtime's own count of real draws).
    pub draws: u64,
    pub load_ms: f64,
}

/// One movie of the layout.
pub struct Slot {
    pub which: Which,
    pub stage: [u32; 2],
    /// Drawn on panels or the screen (its image made with the UI); else made when it is first wanted.
    pub drawn: bool,
    pub srgb_view: bool,
    pub image: Option<Handle<Image>>,
    swf: Option<Vec<u8>>,
    movie: Option<Movie>,
    /// The movie's own frame period (s, `Movie::frame_rate` once loaded; 0 until then): it is stepped no more often.
    period: f64,
    /// Seconds not yet stepped.
    clock: f64,
    /// Its frames' place among the engine's, as a share of its period (`PHASE`).
    phase: f64,
    /// Its last frame's number on the seated clock (`UiMovies::seated_s` in its periods, set off by its phase).
    frame_index: i64,
    /// DataStore writes and messages sent before the movie was loaded, in order (at most `elite_gfx::movie::WAITING_MOST`).
    pending: Vec<Pending>,
    pub failed: Option<String>,
    pub ready: bool,
    pub cost: Cost,
    pub sent: u64,
    pub refused: u64,
    pub traces: u64,
    pub last_traces: VecDeque<String>,
    errors: HashSet<String>,
}

/// The most distinct error texts kept per movie (each logged once); past it errors are counted, not listed.
const ERRORS_MOST: usize = 64;

/// Each movie's frames' place among the engine's, a share of its own period (a movie steps when the seated clock passes
/// (k - phase) periods): the movies are spread over the engine's frames on purpose, the heaviest alone. Measured here a step
/// (with `changed`, seated at rest, 3440 x 1440): the gauge sheet about 2.7 ms, the comms panel 1.2, an open side panel 4.5,
/// the info panel 0.5, the orbital HUD 0.4, the ring 0.35, the rest under 0.1. The engine's frames are not evenly timed, so
/// two movies whose frames start less than an engine frame apart often step together: the comms panel and the orbital HUD
/// (1.6 ms) start half a period after the gauge sheet (2.7), 17 ms, more than an engine frame at 60 a second and over, so
/// those two never meet; the small movies (1.0) start a quarter period after it, and the side panels three quarters (one
/// open at a time).
fn phase(which: Which) -> f64 {
    match which {
        Which::CockpitGaugePanel => 0.0,
        Which::CockpitChatPanel | Which::OrbitalHud => 0.5,
        Which::CockpitHudRing | Which::CockpitInfoPanel | Which::Notifications | Which::TargetSights => 0.75,
        Which::CockpitExternalPanel | Which::CockpitInternalPanel2 | Which::RolePanel3 | Which::FssHud | Which::SaaHud => 0.25,
    }
}

impl Slot {
    pub fn new(which: Which, stage: [u32; 2], drawn: bool, srgb_view: bool, swf: Vec<u8>) -> Slot {
        Slot {
            which,
            stage,
            drawn,
            srgb_view,
            image: None,
            swf: Some(swf),
            movie: None,
            period: 0.0,
            clock: 0.0,
            phase: phase(which),
            frame_index: i64::MIN,
            pending: Vec::new(),
            failed: None,
            ready: false,
            cost: Cost::default(),
            sent: 0,
            refused: 0,
            traces: 0,
            last_traces: VecDeque::new(),
            errors: HashSet::new(),
        }
    }

    pub fn loaded(&self) -> bool {
        self.movie.is_some()
    }

    /// Keeps a write or message for when the movie is loaded (or counts it refused).
    fn keep(&mut self, p: Pending) {
        if self.pending.len() < elite_gfx::movie::WAITING_MOST {
            self.pending.push(p);
        } else {
            self.refused += 1;
        }
    }
}

/// A DataStore write or a message waiting for its movie to be loaded.
enum Pending {
    Store(Vec<String>, String, Value),
    Msg(&'static str, Vec<Value>),
}

fn deliver(movie: &mut Movie, p: Pending) {
    match p {
        Pending::Store(context, property, value) => movie.store(&context.iter().map(String::as_str).collect::<Vec<_>>(), &property, value),
        Pending::Msg(name, args) => movie.send(name, &args),
    }
}

/// The movies (main thread only).
#[derive(Default)]
pub struct UiMovies {
    gpu: Option<Gpu>,
    pub slots: Vec<Slot>,
    fonts: Vec<Font>,
    host: Option<Arc<dyn Host>>,
    /// The seconds the movies have run seated: the one clock their frames are spread on.
    seated_s: f64,
    /// The whole step's time each frame since the last `sw ui state` (`StepTimes`).
    pub step_times: StepTimes,
}

/// A frame's whole `step` (sending, stepping, drawing, draining every movie), in 0.1 ms bins to 25 ms: the spread of the
/// movies' cost over the engine's frames (`sw ui state` prints it and starts again).
#[derive(Clone)]
pub struct StepTimes {
    bins: [u32; 251],
    pub most_ms: f64,
}

impl Default for StepTimes {
    fn default() -> StepTimes {
        StepTimes { bins: [0; 251], most_ms: 0.0 }
    }
}

impl StepTimes {
    fn add(&mut self, ms: f64) {
        self.bins[((ms * 10.0) as usize).min(250)] += 1;
        self.most_ms = self.most_ms.max(ms);
    }

    pub fn frames(&self) -> u32 {
        self.bins.iter().sum()
    }

    /// The share `q` (0..1) of the frames took at most this (ms, to the bin's top).
    pub fn quantile(&self, q: f64) -> f64 {
        let want = (q * self.frames() as f64).ceil().max(1.0) as u32;
        let mut seen = 0;
        for (i, n) in self.bins.iter().enumerate() {
            seen += n;
            if seen >= want {
                return (i + 1) as f64 / 10.0;
            }
        }
        25.0
    }
}

impl UiMovies {
    pub fn new(slots: Vec<Slot>, fonts: Vec<Font>, host: Arc<dyn Host>) -> UiMovies {
        UiMovies { gpu: None, slots, fonts, host: Some(host), seated_s: 0.0, step_times: StepTimes::default() }
    }

    /// Loads the first wanted movie whose target exists on the GPU (one a frame).
    fn load_one(&mut self, wanted: &[Which], handoff: &Handoff, images: &mut Assets<Image>) {
        if self.gpu.is_none() {
            self.gpu = handoff.take_gpu();
        }
        for slot in self.slots.iter_mut().filter(|s| wanted.contains(&s.which) && s.movie.is_none() && s.failed.is_none()) {
            let image = slot.image.get_or_insert_with(|| images.add(target_image(slot.stage, slot.srgb_view)));
            handoff.want(image.id());
        }
        let (Some(gpu), Some(host)) = (self.gpu.as_ref(), self.host.as_ref()) else { return };
        let Some(slot) = self.slots.iter_mut().find(|s| wanted.contains(&s.which) && s.movie.is_none() && s.failed.is_none() && s.image.as_ref().is_some_and(|i| handoff.texture(i.id()).is_some())) else { return };
        let Some(texture) = slot.image.as_ref().and_then(|i| handoff.texture(i.id())) else { return };
        let name = slot.which.name();
        let files = MovieFiles { name: name.to_string(), swf: slot.swf.take().unwrap_or_default(), fonts: self.fonts.clone(), host: host.clone() };
        let t0 = Instant::now();
        match Movie::load(gpu, files, (*texture).clone()) {
            Ok(mut movie) => {
                for p in slot.pending.drain(..) {
                    deliver(&mut movie, p);
                }
                slot.cost.load_ms = t0.elapsed().as_secs_f64() * 1e3;
                slot.period = 1.0 / movie.frame_rate().max(1.0);
                info!("sidewinder ui: {name} loaded ({}x{}, {:.1} ms)", slot.stage[0], slot.stage[1], slot.cost.load_ms);
                slot.movie = Some(movie);
            }
            Err(e) => {
                warn!("sidewinder ui: {name} not loaded: {e}");
                slot.failed = Some(e);
            }
        }
    }
}

/// Each frame while seated: load, send, step, draw, drain (the module's notes); in the grace after (`UiGrace`) step and drain
/// only.
#[allow(clippy::too_many_arguments)]
pub fn step(
    mut movies: NonSendMut<UiMovies>,
    mut messages: ResMut<UiMessages>,
    mut replies: ResMut<UiReplies>,
    mut state: ResMut<UiMovieState>,
    wanted: Res<UiWanted>,
    handoff: Res<Handoff>,
    mut images: ResMut<Assets<Image>>,
    sounds: Option<ResMut<UiSounds>>,
    time: Res<Time<Real>>,
    grace: Res<UiGrace>,
) {
    let start = Instant::now();
    replies.outs.clear();
    state.fresh.clear();
    // (In the grace after the seat, `UiGrace`: no loading, nothing sent (what waits is sent at the next boarding), nothing
    // asked or drawn; the movies step and drain.)
    let seated = grace.seated;
    if seated {
        movies.load_one(&wanted.movies, &handoff, &mut images);
    }
    // (The frame's DataStore writes first, then its messages, each in order: a message that hands a movie a DataStore
    // path finds the nodes written, `elite_ui::feed::Outbox`.)
    let messages = &mut *messages;
    if seated {
        let stores = messages.stores.drain(..).map(|s| (s.movie, Pending::Store(s.context, s.property, s.value)));
        let msgs = messages.msgs.drain(..).map(|m| (m.movie, Pending::Msg(m.name, m.args)));
        for (which, p) in stores.chain(msgs) {
            let Some(slot) = movies.slots.iter_mut().find(|s| s.which == which) else { continue };
            slot.sent += 1;
            match slot.movie.as_mut() {
                Some(m) => deliver(m, p),
                None => slot.keep(p),
            }
        }
    }
    let dt = time.delta_secs_f64();
    movies.seated_s += dt;
    let now = movies.seated_s;
    let mut sounds = sounds;
    for slot in movies.slots.iter_mut().filter(|s| wanted.movies.contains(&s.which)) {
        let which = slot.which;
        let Some(movie) = slot.movie.as_mut() else { continue };
        let t0 = Instant::now();
        // (Stepped once a frame of its own, at its own rate: when the seated clock passes the start of its next frame,
        // its frames' starts set off by its phase. Ruffle runs a movie's frames by its own clock, so a step between two of
        // them would run none. Then, in the runtime's one order, `changed` and at once `draw` when the frame's drawing
        // differs from what the target shows. A movie placed on no piece, the sights and the orbital HUD, is never drawn
        // nor asked: it runs for its messages, events and sounds.)
        slot.clock += dt;
        let index = if slot.period > 0.0 { (now / slot.period + slot.phase).floor() as i64 } else { slot.frame_index.wrapping_add(1) };
        if index != slot.frame_index {
            slot.frame_index = index;
            movie.step(slot.clock);
            slot.clock = 0.0;
            if seated && slot.drawn && movie.changed() {
                movie.draw();
            }
            slot.cost.draws = movie.draws();
        }
        let outs = movie.drain();
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        slot.cost.last_ms = ms;
        slot.cost.total_ms += ms;
        slot.cost.frames += 1;
        if movie.ready() && !slot.ready {
            slot.ready = true;
            state.fresh.push(which);
            state.ready.push(which);
        }
        for out in outs {
            match &out {
                Out::Sound { name, source } => {
                    if let Some(s) = sounds.as_mut() {
                        s.play(interface(which), name, source.as_deref());
                    }
                }
                Out::Trace(text) => {
                    slot.traces += 1;
                    if slot.last_traces.len() == 4 {
                        slot.last_traces.pop_front();
                    }
                    slot.last_traces.push_back(text.clone());
                    if is_error(text) && !slot.errors.contains(text) {
                        if slot.errors.len() < ERRORS_MOST {
                            slot.errors.insert(text.clone());
                            warn!("sidewinder ui: {}: {text}", which.name());
                        } else if slot.errors.len() == ERRORS_MOST {
                            slot.errors.insert(String::new());
                            warn!("sidewinder ui: {}: more than {ERRORS_MOST} kinds of error: the rest are counted, not listed", which.name());
                        }
                    }
                }
                _ => {}
            }
            replies.outs.push((which, out));
        }
    }
    movies.step_times.add(start.elapsed().as_secs_f64() * 1e3);
}
