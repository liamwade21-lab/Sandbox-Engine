//! Reading the ship's sounds: `sounds.json` (`se_core::elite_sound`: parsed, every reference and behaviour checked) and
//! every recording its media table names, each decoded whole by the decoder that will play it before anything can
//! (`mods::check_sound`, `mods::check_wav`), on worker threads. A damaged recording is logged and stays silent; a damaged
//! `sounds.json` leaves the ship silent. Each Wwise Tone Generator is a clip too (T5d-8: `se_core::elite_sound::tone`
//! makes its samples as it plays), under its source plug-in's id; every noise voice shares one noise state, as in the
//! exe.

use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use bevy::audio::AudioSource;
use se_core::elite_sound::data::{Interp, Point};
use se_core::elite_sound::tone::{ToneParams, NOISE_SEED};
use se_core::elite_sound::{parse_sounds, Id, SoundBank, OUTPUT_RATE};

use super::super::CODE;

/// A recording, checked: its bytes and the length the media table gives (the decoder's own end is cut there, so a loop
/// joins sample for sample); or a Tone Generator, whose samples are made as it plays (its own length: `frames` endless).
pub struct Clip {
    pub source: ClipSource,
    pub file: String,
    pub frames: u64,
    pub channels: u16,
    pub rate: u32,
}

/// Where a clip's samples come from.
pub enum ClipSource {
    Recorded(AudioSource),
    /// A Tone Generator's block and the noise state every noise voice shares (`DAT_145ee0fd0`, from its first value in the
    /// exe's image at each load).
    Tone(ToneParams, Arc<AtomicU32>),
}

/// A Wwise Motion Generator's first two channels (the pad's left and right motors: `carried_07`) and its length.
pub struct Rumble {
    pub motors: [Vec<Point>; 2],
    pub length_s: f64,
}

pub struct Loaded {
    pub bank: Arc<SoundBank>,
    pub clips: HashMap<Id, Arc<Clip>>,
    pub rumbles: HashMap<Id, Rumble>,
    /// Recordings refused (file, why): they stay silent.
    pub refused: Vec<(String, String)>,
    pub seconds: f64,
}

fn read(rel: &str) -> Result<Vec<u8>, (String, String)> {
    let path = crate::mods::mod_file(CODE, rel).ok_or_else(|| (rel.to_string(), "missing".to_string()))?;
    std::fs::read(path).map_err(|e| (rel.to_string(), e.to_string()))
}

/// One recording read and checked against its media entry.
fn clip(file: &str, channels: u32, rate: u32, samples: u64) -> Result<Clip, String> {
    let bytes = read(file).map_err(|(_, why)| why)?;
    let (source, facts) = if file.ends_with(".wav") { crate::mods::check_wav(bytes)? } else { crate::mods::check_sound(bytes)? };
    if facts.channels != channels || facts.rate != rate {
        return Err(format!("{} channels at {} Hz, the media table says {channels} at {rate}", facts.channels, facts.rate));
    }
    let decoded = (f64::from(facts.seconds) * f64::from(rate)).round() as u64;
    // (A decoder short of the table by more than a block would cut the recording: refused. Longer is the encoder's padding.)
    if decoded + 64 < samples {
        return Err(format!("decodes to {decoded} samples, the media table says {samples}"));
    }
    Ok(Clip { source: ClipSource::Recorded(source), file: file.to_string(), frames: samples.min(decoded.max(1)), channels: channels as u16, rate })
}

fn rumble(params: &serde_json::Value) -> Option<Rumble> {
    let channel = |i: usize| -> Option<Vec<Point>> {
        params.get("curves")?.get(i)?.as_array()?.iter().map(|p| {
            let a = p.as_array()?;
            Some(Point { x: a.first()?.as_f64()?, y: a.get(1)?.as_f64()?, interp: serde_json::from_value::<Interp>(a.get(2)?.clone()).ok()? })
        }).collect()
    };
    Some(Rumble { motors: [channel(0)?, channel(1)?], length_s: params.get("length_s")?.as_f64()? })
}

/// Everything, or why `sounds.json` can't be used. Run on a worker thread.
pub fn load() -> Result<Loaded, (String, String)> {
    let started = std::time::Instant::now();
    let bank = parse_sounds(&read("sounds.json")?).map_err(|e| ("sounds.json".to_string(), e.to_string()))?;
    let media: Vec<(Id, String, u32, u32, u64)> = bank.file.media.iter().map(|(id, m)| (*id, m.file.clone(), m.channels, m.sample_rate, m.samples)).collect();
    // The decoding shared among threads (it is most of the time).
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8);
    let per = media.len().div_ceil(threads).max(1);
    let results: Vec<(Id, String, Result<Clip, String>)> = std::thread::scope(|s| {
        let jobs: Vec<_> = media.chunks(per).map(|part| s.spawn(move || part.iter().map(|(id, f, c, r, n)| (*id, f.clone(), clip(f, *c, *r, *n))).collect::<Vec<_>>())).collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    });
    let mut clips = HashMap::new();
    let mut refused = Vec::new();
    for (id, file, r) in results {
        match r {
            Ok(c) => {
                clips.insert(id, Arc::new(c));
            }
            Err(why) => refused.push((file, why)),
        }
    }
    let rumbles = bank.file.source_plugins.iter().filter(|(_, p)| p.name == "Wwise Motion Generator").filter_map(|(id, p)| Some((*id, rumble(&p.params)?))).collect();
    // (The bank's load checked every Tone Generator's block.)
    let noise = Arc::new(AtomicU32::new(NOISE_SEED));
    for (id, p) in bank.file.source_plugins.iter().filter(|(_, p)| p.name == "Wwise Tone Generator") {
        if let Ok(t) = ToneParams::from_block(&p.params_hex) {
            let clip = Clip { source: ClipSource::Tone(t, noise.clone()), file: format!("tone {id}"), frames: u64::MAX, channels: 1, rate: OUTPUT_RATE as u32 };
            clips.insert(*id, Arc::new(clip));
        }
    }
    Ok(Loaded { bank: Arc::new(bank), clips, rumbles, refused, seconds: started.elapsed().as_secs_f64() })
}
