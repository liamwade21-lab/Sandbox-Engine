//! Background music: plays SpaceEngine's own `.ogg` tracks from the VFS, one after another.
//!
//! Tracks are read on demand (one ~5 MB file at a time) and played in shuffled order with
//! no immediate repeats. If `music/context.cfg` is present, the pool is the set of tracks it
//! lists under its `Class` entries (menu-only classes `MainMenu` and `Credits` are left out);
//! otherwise every `.ogg` in `music/` is used.
//!
//! As in SE (main-def.cfg `Sound`): consecutive tracks cross-fade over `MixDurationLoop` 5 s, a
//! track chosen by the user (`N`) over `MixDurationGUI` 2 s, the title of each new track shows as
//! an on-screen message (`MusicShowTitle`), and the default volume is `MusicVolume` 20 of 100.
//!
//! Keys: `M` toggles music, `N` skips to the next track (ignored while egui has keyboard focus).
//! Setting `SE_NO_MUSIC` in the environment starts with music disabled.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackMode, PlaybackSettings, Volume};
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use se_core::sc::{self, Value};
use se_core::vfs::Vfs;

use crate::data::SeData;

const MUSIC_DIR: &str = "music";
const CONFIG_PATH: &str = "music/context.cfg";
/// SE's `MusicVolume` 20 (of 100).
pub const DEFAULT_VOLUME: f32 = 0.2;
/// SE's `MixDurationLoop` and `MixDurationGUI` (s).
const MIX_LOOP_S: f32 = 5.0;
const MIX_GUI_S: f32 = 2.0;
/// SE's `MusicShowTitle`.
const SHOW_TITLE: bool = true;

/// Classes in `context.cfg` that describe program screens rather than space scenes.
const SKIPPED_CLASSES: &[&str] = &["MainMenu", "Credits"];

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MusicState {
            enabled: std::env::var_os("SE_NO_MUSIC").is_none(),
            volume: DEFAULT_VOLUME,
            current: None,
        })
        .init_resource::<MusicQueue>()
        .add_systems(Startup, load_library)
        .add_systems(Update, (music_keys, music_driver).chain());
    }
}

/// User-facing playback state.
#[derive(Resource, Debug, Clone)]
pub struct MusicState {
    pub enabled: bool,
    /// Linear volume, 0.0..=1.0.
    pub volume: f32,
    /// Name of the track currently playing (file name without directory).
    pub current: Option<String>,
}

/// The entity that plays a track: its fade (gain 0..1, changing by `rate` per second; a track
/// fading to 0 is removed) and its length.
#[derive(Component)]
pub struct MusicTrack {
    gain: f32,
    rate: f32,
    length: Option<f32>,
}

/// Track list and shuffle bookkeeping (internal).
#[derive(Resource, Default)]
struct MusicQueue {
    /// VFS paths, e.g. `music/Lokijar - Nks.ogg`.
    tracks: Vec<String>,
    /// Indices still to play in this shuffle cycle; the next track is the last element.
    order: Vec<usize>,
    last: Option<usize>,
    rng: u64,
    /// `N` was pressed: cross-fade to the next track over `MIX_GUI_S`.
    skip: bool,
}

impl MusicQueue {
    fn next_rand(&mut self) -> u64 {
        // xorshift64; `rng` is seeded from the clock in `load_library`.
        let mut x = self.rng.max(1);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// Pops the next track index: a fresh shuffle when the cycle is exhausted, and never the
    /// track that just played (when there is more than one track).
    fn pick(&mut self) -> Option<usize> {
        if self.tracks.is_empty() {
            return None;
        }
        if self.order.is_empty() {
            let mut order: Vec<usize> = (0..self.tracks.len()).collect();
            for i in (1..order.len()).rev() {
                let j = (self.next_rand() % (i as u64 + 1)) as usize;
                order.swap(i, j);
            }
            if order.len() > 1 && Some(order[order.len() - 1]) == self.last {
                let n = order.len();
                order.swap(0, n - 1);
            }
            self.order = order;
        }
        self.order.pop()
    }
}

/// Discovers the track pool once at startup.
fn load_library(data: Res<SeData>, mut queue: ResMut<MusicQueue>) {
    let vfs = &data.vfs;
    queue.tracks = track_pool(vfs);
    queue.rng = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
    info!("music: {} tracks available", queue.tracks.len());
}

fn track_pool(vfs: &Vfs) -> Vec<String> {
    let all = vfs.list_ext(MUSIC_DIR, "ogg");
    let mut pool: Vec<String> = Vec::new();
    if let Ok(src) = vfs.read_string(CONFIG_PATH) {
        let block = sc::parse(&src);
        let mut names = Vec::new();
        for class in block.all("Class") {
            if class.name().is_some_and(|n| SKIPPED_CLASSES.iter().any(|s| s.eq_ignore_ascii_case(n))) {
                continue;
            }
            if let Some(files) = class.block.as_ref().and_then(|b| b.get("Files")) {
                collect_strings(&files.args, &mut names);
            }
        }
        for name in names {
            if let Some(path) = all.iter().find(|p| file_name(p).eq_ignore_ascii_case(&name)) {
                if !pool.contains(path) {
                    pool.push(path.clone());
                }
            }
        }
    }
    if pool.is_empty() {
        pool = all;
    }
    pool
}

fn collect_strings(values: &[Value], out: &mut Vec<String>) {
    for v in values {
        match v {
            Value::Str(s) => out.push(s.clone()),
            Value::Tuple(inner) => collect_strings(inner, out),
            _ => {}
        }
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// An Ogg Vorbis file's length (s): the last page's granule position over the sample rate.
pub fn ogg_length(bytes: &[u8]) -> Option<f32> {
    // The identification header: "\x01vorbis", version (4), channels (1), sample rate (4).
    let id = bytes.windows(7).position(|w| w == b"\x01vorbis")?;
    let rate = u32::from_le_bytes(bytes.get(id + 12..id + 16)?.try_into().ok()?);
    let last = bytes.windows(4).rposition(|w| w == b"OggS")?;
    let granule = u64::from_le_bytes(bytes.get(last + 6..last + 14)?.try_into().ok()?);
    (rate > 0 && granule != u64::MAX).then(|| (granule as f64 / rate as f64) as f32)
}

/// Flips music on or off; the driver stops or starts playback accordingly.
pub fn toggle(state: &mut MusicState) {
    state.enabled = !state.enabled;
}

fn music_keys(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<MusicState>, mut queue: ResMut<MusicQueue>, mut egui: EguiContexts) {
    if egui.ctx_mut().map(|c| c.egui_wants_keyboard_input()).unwrap_or(false) {
        return;
    }
    let mods = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::AltLeft, KeyCode::AltRight]);
    if keys.just_pressed(KeyCode::KeyM) && !mods {
        toggle(&mut state);
    }
    if keys.just_pressed(KeyCode::KeyN) && !mods {
        queue.skip = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn music_driver(
    mut commands: Commands,
    time: Res<Time>,
    data: Res<SeData>,
    mut assets: ResMut<Assets<AudioSource>>,
    mut state: ResMut<MusicState>,
    mut queue: ResMut<MusicQueue>,
    mut tracks: Query<(Entity, &mut MusicTrack, Option<&mut AudioSink>)>,
    (mut msg, i18n): (ResMut<crate::messages::Messages>, Res<crate::i18n::I18n>),
) {
    let dt = time.delta_secs();
    // Fades; a track faded out (or finished) is removed.
    let mut playing = None;
    for (e, mut t, sink) in &mut tracks {
        if !state.enabled && t.rate >= 0.0 {
            t.rate = -1.0 / MIX_GUI_S;
        }
        t.gain = (t.gain + t.rate * dt).clamp(0.0, 1.0);
        let Some(mut sink) = sink else {
            if t.rate >= 0.0 {
                playing = Some((e, f32::INFINITY));
            }
            continue;
        };
        sink.set_volume(Volume::Linear(state.volume * t.gain));
        if sink.empty() || (t.rate < 0.0 && t.gain <= 0.0) {
            commands.entity(e).despawn();
            continue;
        }
        if t.rate >= 0.0 {
            let left = t.length.map_or(f32::INFINITY, |l| l - sink.position().as_secs_f32());
            playing = Some((e, left));
        }
    }
    if !state.enabled {
        state.current = None;
        queue.skip = false;
        return;
    }
    // The next track: when none plays, near the end of the one playing, or on `N`.
    let mix = match playing {
        None => 0.0,
        Some(_) if queue.skip => MIX_GUI_S,
        Some((_, left)) if left <= MIX_LOOP_S => MIX_LOOP_S,
        Some(_) => return,
    };
    queue.skip = false;
    if queue.tracks.is_empty() {
        return;
    }
    for _ in 0..queue.tracks.len() {
        let Some(idx) = queue.pick() else { return };
        let path = queue.tracks[idx].clone();
        let bytes = match data.vfs.read(&path) {
            Ok(b) => b,
            Err(e) => {
                warn!("music: cannot read {path}: {e}");
                continue;
            }
        };
        if let Some((e, _)) = playing {
            if let Ok((_, mut t, _)) = tracks.get_mut(e) {
                t.rate = -1.0 / mix.max(0.01);
            }
        }
        let length = ogg_length(&bytes);
        let handle = assets.add(AudioSource { bytes: Arc::from(bytes) });
        let name = file_name(&path).to_string();
        let gain = if mix > 0.0 { 0.0 } else { 1.0 };
        commands.spawn((
            AudioPlayer(handle),
            PlaybackSettings { mode: PlaybackMode::Once, volume: Volume::Linear(state.volume * gain), ..default() },
            MusicTrack { gain, rate: 1.0 / mix.max(0.01), length },
        ));
        info!("music: playing {name}");
        if SHOW_TITLE {
            let title = name.strip_suffix(".ogg").unwrap_or(&name);
            msg.show(format!("{}: {title}", i18n.tr("Music")));
        }
        queue.last = Some(idx);
        state.current = Some(name);
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ogg_length_from_the_last_page() {
        let mut f = vec![0u8; 64];
        f[0..4].copy_from_slice(b"OggS");
        f[28..35].copy_from_slice(b"\x01vorbis");
        f[40..44].copy_from_slice(&44_100u32.to_le_bytes());
        let mut last = b"OggS\x00\x04".to_vec();
        last.extend_from_slice(&(44_100u64 * 90).to_le_bytes());
        f.extend_from_slice(&last);
        assert_eq!(ogg_length(&f), Some(90.0));
    }
}
